"""Build the v1 signal poles: San Francisco traffic-signal poles, mast arms and the hardware
that joins the heads to them, modelled here from published dimensions.

Usage (headless, Blender 4.5):
  blender -b --factory-startup --python scripts/assets/build-signal-poles-v1.py -- \
      [--out public/assets/signal-poles] [--seed 7] [--bake 1024] [--renders DIR --hdri FILE]

Nothing is imported: every part is built in code (bmesh) from the dimensions below and the
game's own signal-hardware constants (the JS export `native/eo-props/data/props-js.json.gz`,
`signalHardware`, `pedHead`, `signalHardwareColour`; the heads' frames from
`public/assets/street-signals/street-signals-v1.provenance.json`).

  * mast pole (`mastPole`): the tapered shaft at the game's radii and height on an 18 in
    square base plate standing on levelling nuts over four 1 1/2 in anchor bolts (washers,
    heavy hex nuts, bolt ends), a weld fillet at the shaft's foot, a reinforced handhole with
    its bolted cover on the far side (local -X; the arm is along local +X), a cast pole-top
    cap, the round concrete foundation flush with the sidewalk (Caltrans RSP ES-7C, below);
  * pedestal and pedestrian posts (`pedestalPole`, `pedPost`): a 4 in steel pipe on a cast
    flared base with four bolt lugs and nuts, a small handhole, a domed post-top cap, a
    smaller foundation (modelling values: the San Francisco pedestal posts' look);
  * the mast arm in three parts so nothing bolted is stretched: `armTube` (unit length along
    +X, the game scales it to the arm's length), `armRoot` (the pole plate welded to the pole
    with its gussets, the arm's flange plate, four cap screws, the weld at the arm's root;
    origin on the pole axis) and `armTip` (the cast end cap; origin at the arm's end);
  * the head mounts: `mastMount{0,1,2}` (a saddle clamped round the arm by two straps and a
    hanger pipe down to the head's top, for arms of three radii), `sideMount{Pipe,Mast}` and
    `frontMount{Pipe,Mast}` (two 1 1/2 in pipe arms from clamp bands on the pole, elbowed into
    the head's top and bottom, for a head beside the pole or in front of it; a pedestal pipe or
    a mast pole), `pedMount{Pipe,Mast}` (the clamshell bands a pedestrian head is bolted to);
  * every part's surfaces are baked with Cycles (object-space position, normal, ambient
    occlusion over the sidewalk, edge pointiness, surface kind) and weathered here in numpy
    into one albedo, normal and metallic-roughness atlas per family (pole, posts, hardware):
    the signal paint chalked and faded by the sun, dust on the up-facing surfaces, chips to the
    galvanised steel at edges, bolts and the foot, rust bled down from every bolt and nut, a
    grime splash up the foot, the foundation's stains: matte, weathered hardware;
  * the level arms at Caltrans RSP ES-7E's diameter for their length (`armLevel{k}`, one per
    `signal-mast-arm-caltrans` row): the rising arm's tube with no rise, tapered by the modelling
    taper from the sheet's outside diameter at the pole;
  * the mast pole cut at its shaft (`mastFoot`, `mastMid`, `mastTop`): each of its levels split
    at MAST_CUT_M (its shaft's uv column edges), so a pole of another height draws its foot
    (base plate, bolts, handhole) and its cap at their true size and only the shaft between
    stretched or shortened;
  * round 5, the corner hardware, two families of their own: the controller cabinet
    (`cabinet`, the McCain MSF60 the city buys, 61 x 36 x 18.5 in, two side-by-side doors with
    latch handles and hinges, louvred intakes, a police door, a hood with its drip lip, on a
    concrete pad; dull mill-finish aluminium, rain-streaked, splashed, graffiti painted out in
    mismatched greys, sticker residue), the No. 5 traffic pull box (`pullBox`, the Christy N30's
    size: a precast lid cast TRAFFIC SIGNAL with two lifting slots, the box's top wall round it)
    and the stiffener channel across an arm sign's back (`signChannel`, unit length);
  * three levels per part (the bolts, nuts, handhole and fittings at level 0 only);
  * every file's digest is written in `signal-poles-v1.provenance.json`.

Frames (glTF Y up): a pole's origin at its foot on the pole axis (the sidewalk), its arm side
+X; an arm part's origin on the arm's axis, +X along the arm; a mount's origin on the pole axis
(or the arm axis) at the head's centre (top) height, +X toward the head.
"""

import bpy
import bmesh
import gzip
import hashlib
import json
import math
import os
import struct
import sys

import numpy as np
from mathutils import Matrix, Vector

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
SCRIPT = os.path.abspath(__file__)
PROPS_JS = os.path.join(ROOT, 'native', 'eo-props', 'data', 'props-js.json.gz')
SIGNALS = os.path.join(ROOT, 'public', 'assets', 'street-signals', 'street-signals-v1.provenance.json')
IN = 0.0254

SOURCES = {
    'caltransEs7e': {
        'title': 'Caltrans Revised Standard Plan RSP ES-7E, Electrical Systems (Signal and Lighting Standard, Case 3 Arm Loading, '
                 'Wind Velocity = 100 mph, Arm Lengths 15 ft to 45 ft)',
        'edition': '2006 Revised Standard Plans, RSP ES-7E dated June 30, 2006 (page 441 of the May 2006 Standard Plans book)',
        'publisher': 'State of California, Department of Transportation',
        'url': 'https://www.oaklandca.gov/files/assets/city/v/1/public-works/documents/standard-details-for-public-works-construction/es-7e.pdf',
        'urlNote': 'a copy of the Caltrans sheet in the City of Oakland\'s standard details',
        'used': 'the signal arm data (projected length, mounting height G, connection height H, angle at the pole, minimum OD at the pole), '
                'through the standards table entry signal-mast-arm-caltrans; the drawing is not reproduced',
    },
    'caMutcd2026': {
        'title': 'California Manual on Uniform Traffic Control Devices, 2026 Edition (FHWA MUTCD 2023 as amended for use in California)',
        'publisher': 'State of California, Department of Transportation',
        'url': 'https://dot.ca.gov/programs/safety-programs/camutcd',
        'used': 'push button height and facing (Section 4I.05, ped-push-button-location), the R10-3e(CA) sign\'s size (Table 2B-1, '
                'ped-push-button-sign) and layout (Figure 2B-27(CA), redrawn), the overhead Street Name sign\'s letter height (Section 2D.45, '
                'street-name-sign-overhead-letter-height)',
    },
    'overpass': {
        'title': 'Overpass 3.0.5 (Bold)', 'license': 'OFL-1.1', 'licenseUrl': 'https://openfontlicense.org',
        'url': 'https://github.com/RedHatOfficial/Overpass',
        'file': 'external/sign-art/overpass (the sign art manifest\'s verified copy)',
        'used': 'the push button sign\'s legends, rasterised into the hardware atlas (the font file is not shipped)',
    },
    'caltransEs7c': {
        'title': 'Caltrans Revised Standard Plan RSP ES-7C, Electrical Systems (Signal and Lighting Standard, Case 1 Arm Loading, '
                 'Wind Velocity = 100 mph, Arm Lengths 15 ft to 30 ft)',
        'edition': '2006 Revised Standard Plans, RSP ES-7C dated June 15, 2007 (page 439 of the May 2006 Standard Plans book)',
        'publisher': 'State of California, Department of Transportation',
        'url': 'https://www.oaklandca.gov/files/assets/city/v/1/public-works/documents/standard-details-for-public-works-construction/es-7c.pdf',
        'urlNote': 'a copy of the Caltrans sheet in the City of Oakland\'s standard details',
        'used': 'dimensions only (Type 16-1-100 / 18-1-100 base plate data, signal arm connection details, CIDH pile diameter); '
                'the drawing is not reproduced',
        'read': {
            'basePlateIn': 18.0, 'basePlateThicknessIn': 1.25, 'anchorBoltDiameterIn': 1.5, 'anchorBoltCircleIn': 17.5,
            'armPlateIn': 12.0, 'armPlateThicknessIn': 1.25, 'polePlateThicknessIn': 1.5, 'capScrewDiameterIn': 1.25,
            'capScrews': 4, 'capScrewCircleIn': 12.0, 'pileDiameterIn': 30.0, 'armOdAtPoleIn': [7.0, 8.0],
            'handhole': 'far side (opposite the arm)',
        },
    },
    # round 5: the corner hardware
    'mccainMsf60': {
        'title': 'SWARCO McCain MSF60 NEMA Cabinet (product page)',
        'publisher': 'SWARCO McCain, Inc.',
        'url': 'https://www.mccain-inc.com/products/cabinets/nema-cabinets/msf60-nema-cabinet',
        'used': 'the controller cabinet\'s overall size, its two side-by-side front doors and its aluminium shell (`cabinet`); the '
                'product\'s drawings are not reproduced (none read): doors, handles, hinges, louvres and hood are modelling values',
        'read': {'heightIn': 61.0, 'widthIn': 36.0, 'depthIn': 18.5, 'doors': 'front (2), side by side, reduced door swing',
                 'material': '5052-H32 aluminium, 0.125 in'},
    },
    'mccainSanFrancisco2014': {
        'title': 'McCain Secures Contract for 180 Traffic Controller Cabinets With the City of San Francisco (press release, May 13, 2014)',
        'publisher': 'McCain, Inc. (Business Wire)',
        'url': 'https://finance.yahoo.com/news/mccain-secures-contract-180-traffic-153000493.html',
        'used': 'that the city buys the MSF60 (its first purchase of 180 of them), chosen for narrow, busy sidewalks',
    },
    'datasfTrafficSignals': {
        'title': 'Traffic Signals (DataSF dataset ybh5-27n2, from the SFMTA signal shop\'s Signals Access database)',
        'publisher': 'City and County of San Francisco, SFMTA', 'license': 'PDDL-1.0',
        'url': 'https://data.sfgov.org/d/ybh5-27n2',
        'used': 'one controller cabinet per signal, and the cabinet sizes the city runs (field CABINET_SI: MSF 596, 2070-MSF-TS2-TYPE-1 207, '
                'MSF65-ATC 78, MSF TALL 15, MSF72 10 of 1507 rows, read 2026-11-27): the MSF family is the common one',
    },
    'christyN30': {
        'title': 'Oldcastle Infrastructure Christy N30 enclosure (product page and the Christy concrete products catalogue)',
        'publisher': 'Oldcastle Infrastructure',
        'url': 'https://oldcastleinfrastructure.com/product/christy-n30/',
        'urlNote': 'the N30 as the No. 5 pull box: City of Lake Elsinore Traffic Signal Specifications (2024), "Christy #5 Box | N30 Box", '
                   'https://www.lake-elsinore.org/DocumentCenter/View/4215',
        'used': 'the No. 5 traffic pull box beside a signal pole (`pullBox`): its opening and its lid; the outside of its top wall from a '
                'distributor\'s overall size (27 1/2 x 18 in)',
        'read': {'openingLengthIn': 24.25, 'openingWidthIn': 13.25, 'lidLengthIn': 24.25, 'lidWidthIn': 13.75, 'lidThicknessIn': 2.0,
                 'outsideLengthIn': 27.5, 'outsideWidthIn': 18.0},
    },
}

# Caltrans RSP ES-7C (SOURCES): the mast pole's base and the signal arm connection.
ES7C = SOURCES['caltransEs7c']['read']
BASE_PLATE = ES7C['basePlateIn'] * IN
BASE_PLATE_T = ES7C['basePlateThicknessIn'] * IN
ANCHOR_D = ES7C['anchorBoltDiameterIn'] * IN
ANCHOR_CIRCLE = ES7C['anchorBoltCircleIn'] * IN
ARM_PLATE = ES7C['armPlateIn'] * IN
ARM_PLATE_T = ES7C['armPlateThicknessIn'] * IN
POLE_PLATE_T = ES7C['polePlateThicknessIn'] * IN
CAP_SCREW_D = ES7C['capScrewDiameterIn'] * IN
CAP_SCREW_CIRCLE = ES7C['capScrewCircleIn'] * IN
PILE_R = ES7C['pileDiameterIn'] * IN / 2

# Modelling values (no primary source read; each says where it comes from).
MODELLING = {
    'heavyHexNut': 'ASME B18.2.2 heavy hex nut proportions: across flats 1.5 x the bolt + 1/8 in, height ~ the bolt',
    'washer': 'ASTM F436 washer proportions: outside diameter 2 x the bolt, 5/32 in thick',
    'levellingGap': 'the base plate on levelling nuts, the gap left open (no grout), as the reference photographs show',
    'handhole': '4 x 7 in opening in a raised 3/4 in frame, centre 19 in over the plate (mast); 2 1/2 x 5 in on the posts',
    'poleCap': 'cast cap 3/8 in over the shaft, domed',
    'pedestalPipe': '4 in nominal steel pipe (4.5 in outside diameter) on a cast flared base, as San Francisco pedestal posts',
    'armRadii': 'the arm tube as the game built it in code (eo-render props_lib: radius 0.09 m at the pole to 0.06 m at the end; '
                'ES-7C gives 7 to 8 in at the pole)',
    'mountPipe': '1 1/2 in nominal pipe (1.9 in outside diameter) bracket arms, as Caltrans side-mount signal brackets; '
                 'a pedestrian head standing off its pole on two such arms (pedReach) from its clamshell',
    'weathering': 'paint fade, dust, chips, rust and grime amounts are artistic, matched by eye to the reference photographs',
    'armTaper': 'the rising arms lose 0.14 in of outside diameter per foot of length from ES-7E\'s diameter at the pole (a common tapered steel '
                'arm taper; no sheet read gives it)',
    'armLevelDiameter': 'a level arm takes ES-7E\'s minimum outside diameter at the pole for its length class and the rising arms\' taper: the '
                        'sheet\'s diameter is the arm\'s strength for its length and loads, rise or none (no SFMTA arm drawing was read)',
    'armRiseShape': 'the arm\'s axis at rise * (1 - (1 - t)^p) over its root, p = tan(angle at the pole) * E / rise (at least 1): it leaves the '
                    'pole at ES-7E\'s angle and ends level, as the sheet draws the arm curving',
    'nameSignHang': 'a Street Name sign hangs level 6 cm under the arm\'s lower end over its width, in the longest stretch of the arm free of '
                    'the heads, on two flat bars 15 cm in from its ends (no SF mounting detail read)',
    'pushButton': 'an accessible push button unit after the San Francisco installations\' look: a housing 12 x 16 x 6.6 cm, a 5 cm button with a '
                  'raised arrow, on a channel clamped to the pole by two bands, the sign over it (sizes are modelling values)',
    'mastCut': 'the mast pole split at 2.0 m and 4.45 m over its foot (its shaft\'s uv column edges): the foot holds the base plate, bolts, '
               'weld and handhole, the top the upper shaft and the cap; a pole of another height draws the shaft between at its own length',
    'pushButtonSign': 'the R10-3e(CA) face drawn here after CA MUTCD 2026 Figure 2B-27(CA) (p. 273) in Overpass: panel layout, legends and the '
                      'arrow as the figure has them, the symbols simple shapes; not the official artwork',
    'cabinetPad': 'the cabinet stands on a cast concrete pad 3 in over the sidewalk, 6 cm wider than the cabinet all round, its top edge eased '
                  '(no SFMTA or Caltrans foundation sheet was read)',
    'cabinetDetail': 'the MSF60\'s two doors raised 4 mm on its front with a 14 mm seam, a latch handle and lock each by the seam, three '
                     'hinges on each outer edge, a louvred intake at each door\'s foot, a police panel door on the right door, a hood '
                     'overhanging 25 to 35 mm with a drip lip, a base flange; the shell mill-finish aluminium gone dull, rain-streaked under '
                     'the hood, splashed at the foot, with graffiti painted out in mismatched grey patches and sticker residue as the '
                     'city\'s cabinets carry (artistic amounts)',
    'pullBoxLegend': 'the No. 5 lid of precast concrete, flush with the sidewalk, cast with TRAFFIC SIGNAL along its length (1 1/4 in '
                     'letters) and two 1/2 x 2 in lifting slots; the box\'s top wall round it',
    'signChannel': 'two aluminium channels (1 1/2 in web, 3/4 in flanges, 0.09 in thick) across the back of an arm\'s Street Name sign '
                   'at a quarter of its height from top and bottom, as the reference photographs show on San Francisco sign backs '
                   '(Lombard and Hyde); no SFMTA sign detail was read',
}

PIPE_R = 4.5 * IN / 2          # pedestal / post pipe (modellingValues pedestalPipe)
MOUNT_PIPE_R = 1.9 * IN / 2    # bracket pipe (modellingValues mountPipe)
ARM_R0, ARM_R1 = 0.09, 0.06    # modellingValues armRadii
MAST_CUT_M = (2.0, 4.45)       # the mast pole's cuts: its shaft's uv column edges (family_pole), modellingValues mastCut
ARM_NOMINAL_M = 6.0            # the arm tube's texture is laid out for a 6 m arm (the game stretches it)
MOUNT_ARM_RADII = (0.0875, 0.0775, 0.0675)
ARM_TAPER_PER_M = 0.14 * IN / 0.3048  # modellingValues armTaper: outside diameter lost per metre of arm
STANDARDS = os.path.join(ROOT, 'scripts', 'world-data', 'sources', 'california-street-standards.json')
# the push button's R10-3e(CA) sign (`ped-push-button-sign`), its bottom over the button's centre
BUTTON_SIGN_M = None  # (width, height), read in main()
BUTTON_HEIGHT = None  # the button's height over the sidewalk (`ped-push-button-location`), read in main()
SIGN_Z0 = 0.075
# Overpass (SIL Open Font License 1.1), the sign art's own copy (`external/sign-art/overpass`)
OVERPASS_DIR = os.path.join(os.environ.get('EO_SIGN_ART_DIR') or os.path.join(ROOT, 'external', 'sign-art'), 'overpass', 'x', 'Overpass-3.0.5', 'webfonts', 'overpass-webfont')
OVERPASS = {'bold': os.path.join(OVERPASS_DIR, 'overpass-bold.ttf')}
POS_SCALE = 0.02  # the position bake's encoding (p * POS_SCALE + 0.5)
# The finishes every part is weathered in (level 0 one file per finish; the far levels, which
# carry no maps, shared): the signal paint (`signalHardwareColour`) chalked and chipped, and bare
# hot-dip galvanising gone to a dull patina. The city has both (the reference photographs: painted
# on Market Street and at 7th and Lincoln, galvanised on Geary at 15th Avenue).
FINISHES = ('painted', 'galvanised')
FINISH_SUFFIX = {'painted': '', 'galvanised': '-galvanised'}


def js_consts():
    d = json.load(gzip.open(PROPS_JS))['consts']
    s, p = d['signalHardware'], d['pedHead']
    colour = d['signalHardwareColour']
    return {
        'rBase': s['poleRadiusBaseM'], 'rTop': s['poleRadiusTopM'], 'mastH': s['mastPoleHeightM'], 'armH': s['mastArmHeightM'],
        'mastHeadC': s['mastHeadCentreM'], 'pedestalH': s['pedestalPoleHeightM'], 'pedestalHeadC': s['pedestalHeadCentreM'],
        'nearMastHeadC': s['nearMastHeadCentreM'], 'lateral': s['poleHeadLateralM'], 'postH': p['postHeightM'],
        'pedC': p['centreM'], 'pedStandoff': p['standoffM'],
        'paint': ((colour >> 16) & 255, (colour >> 8) & 255, colour & 255),
    }


def head_frames():
    """The heads' bounds (street-signals-v1): the vehicle head's backplate half height, the
    pedestrian head's half height."""
    d = json.load(open(SIGNALS))['assets']
    return {'headHalf': d['signalHead']['boundsM']['max'][1], 'pedHalf': d['pedHead']['boundsM']['max'][1], 'pedBack': -d['pedHead']['boundsM']['min'][2]}


# The vehicle head's housing top over its centre (signalHead-v1, measured in its file: the
# housing's top surface over the middle lens at x = 0 is at y 0.50 to 0.525; the backplate
# reaches to boundsM). The hangers end in the housing's top.
HEAD_HOUSING_TOP = 0.50
# The near-side head's offset in front of its pole (eo-props signals.rs: `p + f * 0.2`).
FRONT_REACH = 0.2

# piece ids (a face attribute): what each face is, for the levels and the weathering
P_SHAFT, P_PLATE, P_NUT, P_BOLT, P_WASHER, P_WELD, P_HANDHOLE, P_CAP, P_PAD, P_BASE, P_GUSSET, P_FLANGE, P_SCREW, P_TUBE, P_ENDCAP, P_STRAP, \
    P_SADDLE, P_PIPE, P_BAND, P_FITTER, P_LUG, P_SIGN = range(22)
SMALL = {P_NUT, P_BOLT, P_WASHER, P_WELD, P_HANDHOLE, P_SCREW, P_FITTER}
# surface kinds (material slots while baking)
K_PAINT, K_ZINC, K_CONCRETE, K_CAST, K_SIGN = range(5)
KIND_NAMES = ['paint', 'zinc', 'concrete', 'cast', 'sign']


def args():
    out = {'out': os.path.join(ROOT, 'public', 'assets', 'signal-poles'), 'renders': None, 'hdri': None, 'samples': 64, 'seed': 7,
           'bake': 1024, 'only': None}
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    i = 0
    while i < len(argv):
        k = argv[i].lstrip('-')
        if k not in out:
            raise SystemExit(f'unknown argument {argv[i]}')
        out[k] = argv[i + 1]
        i += 2
    for k in ('samples', 'seed', 'bake'):
        out[k] = int(out[k])
    return out


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        h.update(f.read())
    return h.hexdigest()


# ---------------------------------------------------------------- scene helpers

def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    sc.render.engine = 'CYCLES'
    sc.cycles.device = 'CPU'
    sc.cycles.samples = 1
    sc.render.bake.margin = 6


def select_only(objs, active=None):
    bpy.ops.object.select_all(action='DESELECT')
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = active or objs[0]


def join(objs, name):
    select_only(objs)
    bpy.ops.object.join()
    j = bpy.context.view_layer.objects.active
    j.name = name
    j.data.name = name
    return j


def duplicate(o, name):
    d = o.copy()
    d.data = o.data.copy()
    d.name = name
    bpy.context.scene.collection.objects.link(d)
    return d


def bounds(objs):
    pts = [o.matrix_world @ v.co for o in objs for v in o.data.vertices]
    mn = Vector((min(p[i] for p in pts) for i in range(3)))
    mx = Vector((max(p[i] for p in pts) for i in range(3)))
    return mn, mx


# ---------------------------------------------------------------- geometry (Blender Z up)
#
# Every builder appends to a bmesh `bm` whose face layers carry the piece and the surface kind;
# `new_object` turns it into a mesh object with the four kind material slots.

class Mesh:
    def __init__(self):
        self.bm = bmesh.new()
        self.piece = self.bm.faces.layers.int.new('piece')
        self.kind = self.bm.faces.layers.int.new('kind')
        self.uv = self.bm.loops.layers.uv.new('atlas')
        self.fixed = self.bm.faces.layers.int.new('fixed_uv')  # 1: the uv is laid out already

    def face(self, verts, piece, kind, uvs=None):
        f = self.bm.faces.new(verts)
        f[self.piece] = piece
        f[self.kind] = kind
        if uvs is not None:
            for loop, uv in zip(f.loops, uvs):
                loop[self.uv].uv = uv
            f[self.fixed] = 1
        return f

    def object(self, name):
        me = bpy.data.meshes.new(name)
        bmesh.ops.recalc_face_normals(self.bm, faces=self.bm.faces)
        self.bm.normal_update()
        self.bm.to_mesh(me)
        self.bm.free()
        o = bpy.data.objects.new(name, me)
        bpy.context.scene.collection.objects.link(o)
        for k in KIND_NAMES:
            m = bpy.data.materials.get(k) or bpy.data.materials.new(k)
            me.materials.append(m)
        for p in me.polygons:
            p.material_index = me.attributes['kind'].data[p.index].value
        return o


def ring_pts(c, r, sides, axis='z', a0=0.0):
    """Points of a circle of radius r round centre c, perpendicular to axis."""
    out = []
    for i in range(sides):
        a = a0 + i / sides * math.tau
        ca, sa = math.cos(a), math.sin(a)
        if axis == 'z':
            out.append(Vector((c[0] + ca * r, c[1] + sa * r, c[2])))
        elif axis == 'x':
            out.append(Vector((c[0], c[1] + ca * r, c[2] + sa * r)))
        else:  # y
            out.append(Vector((c[0] + ca * r, c[1], c[2] + sa * r)))
    return out


def tube(m, axis, c0, c1, r0, r1, sides, piece, kind, cap0=False, cap1=False, a0=0.0, uvf=None, rings=1):
    """A (tapered) tube from centre c0 to c1 along an axis ('x', 'y' or 'z'); uvf(i_side, t) -> uv
    lays it out (else it is projected later)."""
    bm = m.bm
    loops = []
    for k in range(rings + 1):
        t = k / rings
        c = [c0[j] + (c1[j] - c0[j]) * t for j in range(3)]
        r = r0 + (r1 - r0) * t
        loops.append([bm.verts.new(p) for p in ring_pts(c, r, sides, axis, a0)])
    for k in range(rings):
        lo, hi = loops[k], loops[k + 1]
        for i in range(sides):
            j = (i + 1) % sides
            uvs = None
            if uvf:
                t0, t1 = k / rings, (k + 1) / rings
                uvs = [uvf(i, t0), uvf(i + 1, t0), uvf(i + 1, t1), uvf(i, t1)]
            m.face([lo[i], lo[j], hi[j], hi[i]], piece, kind, uvs)
    if cap0:
        m.face(list(reversed(loops[0])), piece, kind)
    if cap1:
        m.face(loops[-1], piece, kind)
    return loops


def box(m, mn, mx, piece, kind):
    bm = m.bm
    v = [bm.verts.new((mx[0] if i & 1 else mn[0], mx[1] if i & 2 else mn[1], mx[2] if i & 4 else mn[2])) for i in range(8)]
    for q in ((0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4), (2, 6, 7, 3), (0, 4, 6, 2), (1, 3, 7, 5)):
        m.face([v[i] for i in q], piece, kind)


def chamfer_box(m, c, half, ch, piece, kind):
    """A box with its four vertical edges chamfered by ch (a cast or cut plate seen from above)."""
    bm = m.bm
    hx, hy, hz = half
    pts2 = [(hx - ch, -hy), (hx, -hy + ch), (hx, hy - ch), (hx - ch, hy), (-hx + ch, hy), (-hx, hy - ch), (-hx, -hy + ch), (-hx + ch, -hy)]
    lo = [bm.verts.new((c[0] + x, c[1] + y, c[2] - hz)) for x, y in pts2]
    hi = [bm.verts.new((c[0] + x, c[1] + y, c[2] + hz)) for x, y in pts2]
    n = len(pts2)
    for i in range(n):
        j = (i + 1) % n
        m.face([lo[i], lo[j], hi[j], hi[i]], piece, kind)
    m.face(list(reversed(lo)), piece, kind)
    m.face(hi, piece, kind)


def hex_nut(m, c, af, h, piece=P_NUT, kind=K_ZINC, axis='z', a0=0.0):
    """A hex nut (across flats af, height h) with its chamfered top: 6 sides at across-corners."""
    r = af / math.sqrt(3)
    if axis == 'z':
        c0, c1, c2 = (c[0], c[1], c[2]), (c[0], c[1], c[2] + h * 0.85), (c[0], c[1], c[2] + h)
    else:  # x
        c0, c1, c2 = (c[0], c[1], c[2]), (c[0] + h * 0.85, c[1], c[2]), (c[0] + h, c[1], c[2])
    tube(m, axis, c0, c1, r, r, 6, piece, kind, cap0=True, a0=a0 + math.pi / 6)
    tube(m, axis, c1, c2, r, af / 2 * 0.92, 6, piece, kind, cap1=True, a0=a0 + math.pi / 6)


def washer(m, c, od, t, axis='z', piece=P_WASHER):
    c1 = (c[0], c[1], c[2] + t) if axis == 'z' else (c[0] + t, c[1], c[2])
    tube(m, axis, c, c1, od / 2, od / 2, 12, piece, K_ZINC, cap1=True)


def dome(m, c, r, h, sides, piece, kind, steps=3, axis='z', sign=1.0):
    """A spherical-ish cap of radius r rising h from centre c (along +axis * sign)."""
    bm = m.bm
    rings = []
    for k in range(steps + 1):
        a = k / steps * (math.pi / 2)
        rr, hh = r * math.cos(a), h * math.sin(a) * sign
        if axis == 'z':
            cc = (c[0], c[1], c[2] + hh)
        else:
            cc = (c[0] + hh, c[1], c[2])
        if k == steps:
            rings.append([bm.verts.new(cc)])
        else:
            rings.append([bm.verts.new(p) for p in ring_pts(cc, rr, sides, axis)])
    for k in range(steps):
        lo, hi = rings[k], rings[k + 1]
        for i in range(sides):
            j = (i + 1) % sides
            if len(hi) == 1:
                vs = [lo[i], lo[j], hi[0]]
            else:
                vs = [lo[i], lo[j], hi[j], hi[i]]
            m.face(vs if sign > 0 else list(reversed(vs)), piece, kind)


def torus_elbow(m, c, R, r, a0, a1, plane, sides, segs, piece, kind):
    """A pipe bend: centre c, bend radius R, pipe radius r, from angle a0 to a1 in plane 'xz'
    (angle 0 along +x, pi/2 along +z)."""
    bm = m.bm
    loops = []
    for k in range(segs + 1):
        a = a0 + (a1 - a0) * k / segs
        ca, sa = math.cos(a), math.sin(a)
        centre = Vector((c[0] + R * ca, c[1], c[2] + R * sa))
        radial = Vector((ca, 0, sa))
        side = Vector((0, 1, 0))
        loops.append([bm.verts.new(centre + (radial * math.cos(t) + side * math.sin(t)) * r) for t in (i / sides * math.tau for i in range(sides))])
    for k in range(segs):
        for i in range(sides):
            j = (i + 1) % sides
            vs = [loops[k][i], loops[k][j], loops[k + 1][j], loops[k + 1][i]]
            m.face(vs if a1 > a0 else list(reversed(vs)), piece, kind)


# ---------------------------------------------------------------- the parts

class Shaft:
    """A pole shaft's UV columns: z ranges laid out side by side, each column `w` wide."""

    def __init__(self, cols, h_m):
        self.cols = cols  # [(z0, z1, u0, u1, v0, v1)]
        self.h = h_m

    def uv(self, z, a):
        """uv of the shaft at height z and angle a (0..1 round the pole)."""
        for (z0, z1, u0, u1, v0, v1) in self.cols:
            if z <= z1 + 1e-6:
                t = (z - z0) / (z1 - z0)
                return (u0 + (u1 - u0) * a, v0 + (v1 - v0) * t)
        z0, z1, u0, u1, v0, v1 = self.cols[-1]
        return (u0 + (u1 - u0) * a, v1)


def shaft(m, sh, z0, z1, rf, sides, cuts):
    """The shaft from z0 to z1, radius rf(z), split at the UV columns' edges (and at cuts)."""
    zs = sorted(set([z0, z1] + [c for c in cuts if z0 < c < z1] + [c[1] for c in sh.cols if z0 < c[1] < z1]))
    bm = m.bm
    for zi in range(len(zs) - 1):
        za, zb = zs[zi], zs[zi + 1]
        # columns split the ring's uv: a ring's verts are per segment (the uv seam at a = 0 / 1)
        lo = [bm.verts.new(p) for p in ring_pts((0, 0, za), rf(za), sides, 'z')]
        hi = [bm.verts.new(p) for p in ring_pts((0, 0, zb), rf(zb), sides, 'z')]
        zmid = (za + zb) / 2
        col = next(c for c in sh.cols if zmid <= c[1] + 1e-6)
        for i in range(sides):
            j = (i + 1) % sides
            a0, a1 = i / sides, (i + 1) / sides

            def uv(z, a):
                t = (z - col[0]) / (col[1] - col[0])
                return (col[2] + (col[3] - col[2]) * a, col[4] + (col[5] - col[4]) * t)
            m.face([lo[i], lo[j], hi[j], hi[i]], P_SHAFT, K_PAINT, [uv(za, a0), uv(za, a1), uv(zb, a1), uv(zb, a0)])


def mast_pole(c, sh, lod):
    """The mast pole (lod 0..2). c: the JS constants."""
    m = Mesh()
    H = c['mastH']
    rf = lambda z: c['rBase'] + (c['rTop'] - c['rBase']) * z / H
    pad_top = 0.006
    lev_nut_h = ANCHOR_D * 1.0
    plate_z0 = pad_top + lev_nut_h
    plate_z1 = plate_z0 + BASE_PLATE_T
    sides = (32, 12, 6)[lod]
    shaft(m, sh, plate_z1 if lod < 2 else 0.0, H - 0.03, rf, sides, [0.25, 0.7, 1.2])
    if lod == 2:
        tube(m, 'z', (0, 0, H - 0.03), (0, 0, H), rf(H) + 0.008, rf(H) + 0.008, sides, P_CAP, K_CAST, cap1=True)
        return m
    # the cap: a cast collar over the shaft's top, domed
    tube(m, 'z', (0, 0, H - 0.05), (0, 0, H), rf(H) + 0.0095, rf(H) + 0.0095, sides, P_CAP, K_CAST, cap0=True)
    dome(m, (0, 0, H), rf(H) + 0.0095, 0.03, sides, P_CAP, K_CAST, steps=3 if lod == 0 else 1)
    # the base plate on its levelling nuts, the foundation
    chamfer_box(m, (0, 0, (plate_z0 + plate_z1) / 2), (BASE_PLATE / 2, BASE_PLATE / 2, BASE_PLATE_T / 2), 0.012, P_PLATE, K_PAINT)
    tube(m, 'z', (0, 0, -0.06), (0, 0, pad_top), PILE_R, PILE_R - 0.004, (48, 24)[lod], P_PAD, K_CONCRETE, cap1=True)
    if lod == 1:
        return m
    # the weld fillet at the shaft's foot
    tube(m, 'z', (0, 0, plate_z1), (0, 0, plate_z1 + 0.014), rf(plate_z1) + 0.011, rf(plate_z1) + 0.001, sides, P_WELD, K_PAINT)
    # anchor bolts: levelling nut, plate, washer, nut, the bolt's end
    af, nh = 1.5 * ANCHOR_D + 0.125 * IN, ANCHOR_D * 0.98
    for k in range(4):
        a = math.pi / 4 + k * math.pi / 2
        x, y = math.cos(a) * ANCHOR_CIRCLE / 2, math.sin(a) * ANCHOR_CIRCLE / 2
        hex_nut(m, (x, y, pad_top), af, lev_nut_h - 0.001, a0=k * 0.3)
        washer(m, (x, y, plate_z1), 2 * ANCHOR_D, 5 / 32 * IN)
        hex_nut(m, (x, y, plate_z1 + 5 / 32 * IN), af, nh, a0=0.2 + k * 0.5)
        top = plate_z1 + 5 / 32 * IN + nh
        tube(m, 'z', (x, y, top), (x, y, top + 0.022), ANCHOR_D / 2, ANCHOR_D / 2 - 0.002, 10, P_BOLT, K_ZINC, cap1=True)
    # the handhole on the far side (-X): a raised frame, the cover, two screws
    hz = 0.075 + 19 * IN * 0.5 + 0.24
    r = rf(hz)
    fw, fh, fr = 4 * IN + 2 * 0.75 * IN, 7 * IN + 2 * 0.75 * IN, 0.006
    box(m, (-(r + fr), -fw / 2, hz - fh / 2), (-(r - 0.03), -fw / 2 + 0.75 * IN, hz + fh / 2), P_HANDHOLE, K_PAINT)
    box(m, (-(r + fr), fw / 2 - 0.75 * IN, hz - fh / 2), (-(r - 0.03), fw / 2, hz + fh / 2), P_HANDHOLE, K_PAINT)
    box(m, (-(r + fr), -fw / 2 + 0.75 * IN, hz - fh / 2), (-(r - 0.03), fw / 2 - 0.75 * IN, hz - fh / 2 + 0.75 * IN), P_HANDHOLE, K_PAINT)
    box(m, (-(r + fr), -fw / 2 + 0.75 * IN, hz + fh / 2 - 0.75 * IN), (-(r - 0.03), fw / 2 - 0.75 * IN, hz + fh / 2), P_HANDHOLE, K_PAINT)
    box(m, (-(r + fr - 0.004), -fw / 2 + 0.75 * IN, hz - fh / 2 + 0.75 * IN), (-(r - 0.02), fw / 2 - 0.75 * IN, hz + fh / 2 - 0.75 * IN), P_HANDHOLE, K_PAINT)
    for s in (-1, 1):
        tube(m, 'x', (-(r + fr - 0.004), 0, hz + s * (fh / 2 - 0.75 * IN - 0.018)), (-(r + fr + 0.002), 0, hz + s * (fh / 2 - 0.75 * IN - 0.018)),
             0.007, 0.006, 8, P_SCREW, K_ZINC, cap1=True)
    return m


def pedestal(c, sh, height, lod):
    """A pedestal or pedestrian post: the pipe on its cast flared base."""
    m = Mesh()
    H = height
    rf = lambda z: PIPE_R
    pad_top = 0.006
    base_h = 0.20
    sides = (24, 12, 6)[lod]
    shaft(m, sh, base_h - 0.02 if lod < 2 else 0.0, H - 0.04, rf, sides, [0.6, 1.2])
    if lod == 2:
        tube(m, 'z', (0, 0, H - 0.04), (0, 0, H), PIPE_R + 0.008, PIPE_R + 0.008, sides, P_CAP, K_CAST, cap1=True)
        return m
    # post-top cap: collar and dome, a small knob
    tube(m, 'z', (0, 0, H - 0.05), (0, 0, H), PIPE_R + 0.007, PIPE_R + 0.007, sides, P_CAP, K_CAST, cap0=True)
    dome(m, (0, 0, H), PIPE_R + 0.007, 0.035, sides, P_CAP, K_CAST, steps=3 if lod == 0 else 1)
    if lod == 0:
        tube(m, 'z', (0, 0, H + 0.03), (0, 0, H + 0.045), 0.012, 0.016, 10, P_CAP, K_CAST)
        dome(m, (0, 0, H + 0.045), 0.016, 0.014, 10, P_CAP, K_CAST, steps=2)
    # the cast base: a square foot with chamfered corners, a flared skirt to the pipe, a ring
    foot = 0.15
    chamfer_box(m, (0, 0, pad_top + 0.014), (foot, foot, 0.014), 0.04, P_BASE, K_CAST)
    prof = [(0.028, 0.135), (0.05, 0.118), (0.08, 0.098), (0.12, 0.08), (0.16, 0.07), (base_h, PIPE_R + 0.008)]
    bsides = (32, 12)[lod]
    for (za, ra), (zb, rb) in zip(prof, prof[1:]):
        tube(m, 'z', (0, 0, za), (0, 0, zb), ra, rb, bsides, P_BASE, K_CAST)
    tube(m, 'z', (0, 0, base_h), (0, 0, base_h + 0.012), PIPE_R + 0.008, PIPE_R + 0.004, bsides, P_BASE, K_CAST, cap1=False)
    tube(m, 'z', (0, 0, -0.06), (0, 0, pad_top), 0.24, 0.236, (36, 18)[lod], P_PAD, K_CONCRETE, cap1=True)
    if lod == 1:
        return m
    # four bolt lugs on the foot's corners with nuts and bolt ends
    for k in range(4):
        a = math.pi / 4 + k * math.pi / 2
        x, y = math.cos(a) * 0.15, math.sin(a) * 0.15
        tube(m, 'z', (x, y, pad_top + 0.028), (x, y, pad_top + 0.04), 0.025, 0.022, 12, P_LUG, K_CAST, cap1=True)
        hex_nut(m, (x, y, pad_top + 0.04), 1.5 * 0.75 * IN + 0.125 * IN, 0.75 * IN, a0=k * 0.4)
        top = pad_top + 0.04 + 0.75 * IN
        tube(m, 'z', (x, y, top), (x, y, top + 0.012), 0.75 * IN / 2, 0.75 * IN / 2 - 0.001, 8, P_BOLT, K_ZINC, cap1=True)
    # the handhole (-X), small
    hz = 0.36
    fw, fh = 2.5 * IN + 2 * 0.5 * IN, 5 * IN + 2 * 0.5 * IN
    rr = PIPE_R
    box(m, (-(rr + 0.008), -fw / 2, hz - fh / 2), (-(rr - 0.02), fw / 2, hz + fh / 2), P_HANDHOLE, K_PAINT)
    for s in (-1, 1):
        tube(m, 'x', (-(rr + 0.008), 0, hz + s * (fh / 2 - 0.012)), (-(rr + 0.0125), 0, hz + s * (fh / 2 - 0.012)), 0.005, 0.0045, 8, P_SCREW, K_ZINC, cap1=True)
    return m


def arm_tube(lod, uvrect):
    """The arm tube, unit length along +X (the game scales X to the arm's length)."""
    m = Mesh()
    sides = (20, 10, 5)[lod]
    u0, v0, u1, v1 = uvrect
    uvf = lambda i, t: (u0 + (u1 - u0) * t, v0 + (v1 - v0) * (i / sides))
    tube(m, 'x', (0, 0, 0), (1, 0, 0), ARM_R0, ARM_R1, sides, P_TUBE, K_PAINT, uvf=uvf, rings=(6, 2, 1)[lod], a0=math.pi / 2)
    return m


def arm_root(c, lod):
    """The arm's connection to the pole (ES-7C): the pole plate on its gussets, the arm's
    flange, four cap screws, the root weld. Origin on the pole axis at the arm's axis."""
    m = Mesh()
    H = c['mastH']
    ra = c['rBase'] + (c['rTop'] - c['rBase']) * c['armH'] / H
    x0 = ra + 0.035  # the pole plate's back face
    hp = ARM_PLATE / 2
    # gussets (1/4 in plates top, bottom and sides) from inside the pole to the plate
    t = 0.25 * IN
    if lod == 0:
        box(m, (ra * 0.5, -hp + 0.02, hp - 0.02 - t), (x0, hp - 0.02, hp - 0.02), P_GUSSET, K_PAINT)
        box(m, (ra * 0.5, -hp + 0.02, -hp + 0.02), (x0, hp - 0.02, -hp + 0.02 + t), P_GUSSET, K_PAINT)
        box(m, (ra * 0.5, -hp + 0.02, -hp + 0.02), (x0, -hp + 0.02 + t, hp - 0.02), P_GUSSET, K_PAINT)
        box(m, (ra * 0.5, hp - 0.02 - t, -hp + 0.02), (x0, hp - 0.02, hp - 0.02), P_GUSSET, K_PAINT)
    else:
        box(m, (ra * 0.5, -hp + 0.02, -hp + 0.02), (x0, hp - 0.02, hp - 0.02), P_GUSSET, K_PAINT)
    chamfer_box(m, (x0 + POLE_PLATE_T / 2, 0, 0), (POLE_PLATE_T / 2, hp, hp), 0.006, P_PLATE, K_PAINT)
    # (the chamfered box's chamfer is along its z edges; the plate's are its x edges: fine at 6 mm)
    x1 = x0 + POLE_PLATE_T
    box(m, (x1, -hp + 0.004, -hp + 0.004), (x1 + ARM_PLATE_T, hp - 0.004, hp - 0.004), P_FLANGE, K_PAINT)
    x2 = x1 + ARM_PLATE_T
    if lod == 0:
        tube(m, 'x', (x2, 0, 0), (x2 + 0.02, 0, 0), ARM_R0 + 0.014, ARM_R0 + 0.001, 20, P_WELD, K_PAINT, a0=math.pi / 2)
        af = 1.5 * CAP_SCREW_D + 0.125 * IN
        for k in range(4):
            a = math.pi / 4 + k * math.pi / 2
            y, z = math.cos(a) * CAP_SCREW_CIRCLE / 2, math.sin(a) * CAP_SCREW_CIRCLE / 2
            washer(m, (x2, y, z), 2 * CAP_SCREW_D, 5 / 32 * IN, axis='x')
            hex_nut(m, (x2 + 5 / 32 * IN, y, z), af, 0.625 * CAP_SCREW_D, piece=P_SCREW, axis='x', a0=k * 0.35)
    return m


def arm_tip(lod):
    """The cast end cap at the arm's end (origin there; the tube's radius ARM_R1)."""
    m = Mesh()
    sides = (20, 10)[min(lod, 1)]
    tube(m, 'x', (-0.07, 0, 0), (0.0, 0, 0), ARM_R1 + 0.006, ARM_R1 + 0.006, sides, P_ENDCAP, K_CAST, cap0=True, a0=math.pi / 2)
    if lod == 0:
        tube(m, 'x', (-0.075, 0, 0), (-0.07, 0, 0), ARM_R1 + 0.001, ARM_R1 + 0.006, sides, P_ENDCAP, K_CAST, a0=math.pi / 2)
    dome(m, (0, 0, 0), ARM_R1 + 0.006, 0.025, sides, P_ENDCAP, K_CAST, steps=3 if lod == 0 else 1, axis='x')
    return m


def mast_mount(arm_r, lod):
    """A head hung from the arm: two straps round the arm (radius arm_r), a saddle under it, the
    hanger pipe down into the head's top with its slip fitter. Origin on the arm axis; the head's
    top is HEAD_TOP_BELOW_ARM under it."""
    m = Mesh()
    c = js_consts()
    top_below = (c['armH'] - c['mastHeadC']) - HEAD_HOUSING_TOP  # arm axis to the housing's top
    sides = (16, 8)[min(lod, 1)]
    for s in (-1, 1):
        x = s * 0.06
        tube(m, 'x', (x - 0.0125, 0, 0), (x + 0.0125, 0, 0), arm_r + 0.006, arm_r + 0.006, sides, P_STRAP, K_ZINC, a0=math.pi / 2)
        if lod == 0:
            for e in (x - 0.0125, x + 0.0125):
                tube(m, 'x', (e, 0, 0), (e + 0.0001, 0, 0), arm_r + 0.0005, arm_r + 0.006, sides, P_STRAP, K_ZINC, a0=math.pi / 2)
    # the saddle under the arm and its two ears the straps run through
    box(m, (-0.085, -0.04, -arm_r - 0.03), (0.085, 0.04, -arm_r + 0.012), P_SADDLE, K_CAST)
    # the hanger pipe and its slip fitter on the head's top
    tube(m, 'z', (0, 0, -top_below - 0.01), (0, 0, -arm_r - 0.03), MOUNT_PIPE_R, MOUNT_PIPE_R, sides, P_PIPE, K_PAINT)
    if lod == 0:
        tube(m, 'z', (0, 0, -top_below - 0.012), (0, 0, -top_below + 0.035), 0.038, 0.034, sides, P_FITTER, K_CAST, cap1=True)
        for s in (-1, 1):
            tube(m, 'y', (0, s * 0.036, -top_below + 0.012), (0, s * 0.046, -top_below + 0.012), 0.006, 0.006, 6, P_SCREW, K_ZINC, cap1=True)
    return m


def side_mount(band_r, reach, lod, half):
    """Two pipe arms from clamp bands on a pole (radius band_r) out `reach` to the head's
    centre, elbowed into its top and bottom (the head's half height `half`, housing top
    HEAD_HOUSING_TOP). Origin on the pole axis at the head's centre; +X toward the head."""
    m = Mesh()
    sides = (12, 6)[min(lod, 1)]
    y_pipe = half + MOUNT_PIPE_R + 0.02
    bend = 0.055
    for s in (1, -1):
        z = s * y_pipe
        # the clamp band round the pole and its boss
        tube(m, 'z', (0, 0, z - 0.03), (0, 0, z + 0.03), band_r + 0.007, band_r + 0.007, (24, 12)[min(lod, 1)], P_BAND, K_PAINT)
        if lod == 0:
            for e in (z - 0.03, z + 0.03):
                tube(m, 'z', (0, 0, e), (0, 0, e + 0.0001), band_r + 0.0005, band_r + 0.007, 24, P_BAND, K_PAINT)
        tube(m, 'x', (band_r, 0, z), (band_r + 0.045, 0, z), 0.034, 0.03, sides, P_FITTER, K_CAST, a0=math.pi / 2)
        # the pipe arm to the bend, the bend, the drop into the head
        x_end = max(reach - bend, band_r + 0.05)
        tube(m, 'x', (band_r + 0.04, 0, z), (x_end, 0, z), MOUNT_PIPE_R, MOUNT_PIPE_R, sides, P_PIPE, K_PAINT, a0=math.pi / 2)
        if s > 0:
            torus_elbow(m, (x_end, 0, z - bend), bend, MOUNT_PIPE_R, math.pi / 2, 0.0, 'xz', sides, (6, 3)[min(lod, 1)], P_PIPE, K_PAINT)
            tube(m, 'z', (x_end + bend, 0, HEAD_HOUSING_TOP - 0.02), (x_end + bend, 0, z - bend), MOUNT_PIPE_R, MOUNT_PIPE_R, sides, P_PIPE, K_PAINT)
            if lod == 0:
                tube(m, 'z', (x_end + bend, 0, HEAD_HOUSING_TOP - 0.01), (x_end + bend, 0, HEAD_HOUSING_TOP + 0.04), 0.036, 0.032, sides, P_FITTER, K_CAST, cap1=True)
        else:
            torus_elbow(m, (x_end, 0, z + bend), bend, MOUNT_PIPE_R, -math.pi / 2, 0.0, 'xz', sides, (6, 3)[min(lod, 1)], P_PIPE, K_PAINT)
            tube(m, 'z', (x_end + bend, 0, z + bend), (x_end + bend, 0, -HEAD_HOUSING_TOP + 0.02), MOUNT_PIPE_R, MOUNT_PIPE_R, sides, P_PIPE, K_PAINT)
            if lod == 0:
                tube(m, 'z', (x_end + bend, 0, -HEAD_HOUSING_TOP - 0.04), (x_end + bend, 0, -HEAD_HOUSING_TOP + 0.01), 0.032, 0.036, sides, P_FITTER, K_CAST, cap0=True)
    return m


def ped_mount(band_r, half, lod):
    """The clamshell a pedestrian head is bolted to: two bands round the pole and the hinge
    plate between them on the head's side (+X). Origin on the pole axis at the head's centre."""
    m = Mesh()
    for s in (1, -1):
        z = s * (half - 0.05)
        tube(m, 'z', (0, 0, z - 0.025), (0, 0, z + 0.025), band_r + 0.007, band_r + 0.007, (24, 12)[min(lod, 1)], P_BAND, K_CAST)
        if lod == 0:
            for e in (z - 0.025, z + 0.025):
                tube(m, 'z', (0, 0, e), (0, 0, e + 0.0001), band_r + 0.0005, band_r + 0.007, 24, P_BAND, K_CAST)
    box(m, (band_r - 0.005, -0.05, -(half - 0.05) - 0.025), (band_r + 0.03, 0.05, (half - 0.05) + 0.025), P_SADDLE, K_CAST)
    return m


def ped_reach(half, lod):
    """The two pipe arms a pedestrian head standing off its pole is carried on (with the
    clamshell on the pole): unit length along +X from the pole axis (the game stretches X to the
    head), at the clamshell bands' heights. Origin on the pole axis at the head's centre."""
    m = Mesh()
    sides = (12, 6)[min(lod, 1)]
    for s in (1, -1):
        z = s * (half - 0.05)
        tube(m, 'x', (0.0, 0, z), (1.0, 0, z), MOUNT_PIPE_R, MOUNT_PIPE_R, sides, P_PIPE, K_PAINT, cap1=True, a0=math.pi / 2)
    return m


# ---------------------------------------------------------------- round 2: rising arms, sign brackets, push buttons

def arm_classes():
    """Caltrans RSP ES-7E's signal arm rows (`signal-mast-arm-caltrans` in the standards table):
    per arm length E its rise G - H, its angle at the pole, its outside diameter at the pole;
    the profile's exponent as eo-props `signals::ArmClass::exponent` has it; the tip radius by
    the modelling taper (MODELLING['armTaper'])."""
    d = json.load(open(STANDARDS))
    e = next(x for x in d['entries'] if x['id'] == 'signal-mast-arm-caltrans')['metricValue']
    out = []
    for E, G, H, deg, od in zip(e['projectedLength'], e['mountingHeight'], e['armConnectionHeight'], e['riseAngleDeg'], e['minOdAtPole']):
        rise = G - H
        p = max(1.0, math.tan(math.radians(deg)) * E / rise)
        r0 = od / 2
        r1 = max(r0 - ARM_TAPER_PER_M * E / 2, 0.03)
        out.append({'length': E, 'rise': rise, 'angleDeg': deg, 'od': od, 'p': p, 'r0': r0, 'r1': r1})
    return out


def arm_rise(cl, lod, uvrect):
    """A rising arm of its class's length along +X, its axis at rise * (1 - (1 - t)^p) over the
    root (Blender +Z), tapered from r0 to r1; uv along its length and round it as the level
    tube's (`arm_tube`), so both sample the same strip of the atlas."""
    m = Mesh()
    bm = m.bm
    E, R, p, r0, r1 = cl['length'], cl['rise'], cl['p'], cl['r0'], cl['r1']
    sides, segs = (20, 10, 6)[lod], (24, 8, 4)[lod]
    u0, v0, u1, v1 = uvrect
    loops = []
    for j in range(segs + 1):
        t = j / segs
        x, z = E * t, R * (1 - (1 - t) ** p)
        slope = R * p * (1 - t) ** (p - 1) / E if t < 1 else 0.0
        ph = math.atan(slope)
        up = Vector((-math.sin(ph), 0.0, math.cos(ph)))
        side = Vector((0.0, 1.0, 0.0))
        r = r0 + (r1 - r0) * t
        c = Vector((x, 0.0, z))
        loops.append([bm.verts.new(c + (side * math.cos(a) + up * math.sin(a)) * r) for a in (math.pi / 2 + i / sides * math.tau for i in range(sides))])
    for j in range(segs):
        for i in range(sides):
            k = (i + 1) % sides
            t0, t1 = j / segs, (j + 1) / segs
            uv = lambda ii, t: (u0 + (u1 - u0) * t, v0 + (v1 - v0) * (ii / sides))
            m.face([loops[j][i], loops[j][k], loops[j + 1][k], loops[j + 1][i]], P_TUBE, K_PAINT, [uv(i, t0), uv(i + 1, t0), uv(i + 1, t1), uv(i, t1)])
    return m


def sign_bracket(lod):
    """A Street Name sign's hanger: a flat bar from the arm's axis (y = 0) down to y = -1 (the
    game stretches it to the sign's top), its width along the arm (+X)."""
    m = Mesh()
    box(m, (-0.025, -0.004, -1.0), (0.025, 0.004, 0.0), P_STRAP, K_ZINC)
    return m


def push_button(band_r, arrow, lod, sign_rect):
    """A pedestrian push button on a pole (radius band_r) with its R10-3e(CA) sign above it:
    two bands round the pole, a channel on its +X face carrying the sign and under it the
    button's housing with the button, its raised arrow along the part's +Z (arrow 1) or -Z (-1)
    in the game's frame (glTF +Z is Blender's -Y). Origin on the pole axis at the
    button's centre; +X the way its face looks. The sign's face takes `sign_rect` of the atlas."""
    m = Mesh()
    sw, sh = BUTTON_SIGN_M
    cx = band_r
    sides = (20, 10)[min(lod, 1)]
    for z in (-0.06, SIGN_Z0 + sh - 0.06):
        tube(m, 'z', (0, 0, z - 0.012), (0, 0, z + 0.012), band_r + 0.005, band_r + 0.005, (24, 12)[min(lod, 1)], P_BAND, K_ZINC)
    box(m, (cx - 0.004, -0.04, -0.11), (cx + 0.02, 0.04, SIGN_Z0 + sh), P_SADDLE, K_CAST)
    # the housing and the button
    chamfer_box(m, (cx + 0.052, 0.0, -0.02), (0.033, 0.06, 0.08), 0.008, P_HANDHOLE, K_CAST)
    tube(m, 'x', (cx + 0.085, 0, 0.0), (cx + 0.096, 0, 0.0), 0.026, 0.024, sides, P_SCREW, K_ZINC, cap1=True)
    if lod == 0:
        # the raised arrow on the button: a bar and a head
        xa = cx + 0.0965
        bm = m.bm
        def flat(pts, piece):
            vs = [bm.verts.new((xa, y, z)) for y, z in pts]
            m.face(vs, piece, K_ZINC)
        s = -arrow
        flat([(-0.013 * s, -0.003), (0.004 * s, -0.003), (0.004 * s, 0.003), (-0.013 * s, 0.003)][::s], P_SCREW)
        flat([(0.004 * s, -0.009), (0.015 * s, 0.0), (0.004 * s, 0.009)][::s], P_SCREW)
        # the speaker grille slots over the button
        for k in range(3):
            box(m, (cx + 0.085, -0.02, 0.032 + 0.008 * k), (cx + 0.0855, 0.02, 0.035 + 0.008 * k), P_SCREW, K_CAST)
    # the sign: its face (+X) on the atlas's sign rectangle (u to the viewer's right, Blender +Y; v up)
    bm = m.bm
    x0, x1 = cx + 0.02, cx + 0.023
    y0, y1, z0, z1 = -sw / 2, sw / 2, SIGN_Z0, SIGN_Z0 + sh
    u0, v0, u1, v1 = sign_rect
    v = [bm.verts.new(p) for p in ((x1, y0, z0), (x1, y1, z0), (x1, y1, z1), (x1, y0, z1))]
    m.face(v, P_SIGN, K_SIGN, [(u0, v0), (u1, v0), (u1, v1), (u0, v1)])
    w = [bm.verts.new(p) for p in ((x0, y0, z0), (x0, y1, z0), (x0, y1, z1), (x0, y0, z1))]
    for a, b in ((0, 1), (1, 2), (2, 3), (3, 0)):
        m.face([w[a], w[b], v[b], v[a]], P_PLATE, K_ZINC)
    m.face([w[3], w[2], w[1], w[0]], P_PLATE, K_ZINC)
    return m


def render_button_sign(arrow, px_w, px_h, tdir):
    """The R10-3e(CA) sign (CA MUTCD 2026 Figure 2B-27(CA), 9 x 15 in) drawn here in Overpass on
    a white face, its arrow pointing to the viewer's right (arrow 1) or left (-1): the four
    panels (walk: START CROSSING; flashing hand and timer: MAY START, TIME REMAINING; steady
    hand: DON'T CROSS; PUSH BUTTON TO CROSS). Not the sheet's artwork (none read): the
    symbols are simple shapes. Returns linear RGB (rows from the bottom), px_h x px_w."""
    keep = set(bpy.data.objects)
    sc = bpy.context.scene
    W, H = 9.0, 15.0  # inches; the plane from (0, 0) to (W, H)
    font = bpy.data.fonts.load(OVERPASS['bold'])
    mats = {}

    def mat(name, rgb):
        if name in mats:
            return mats[name]
        mm = bpy.data.materials.new('sign-' + name)
        mm.use_nodes = True
        nt = mm.node_tree
        for x in list(nt.nodes):
            if x.type != 'OUTPUT_MATERIAL':
                nt.nodes.remove(x)
        em = nt.nodes.new('ShaderNodeEmission')
        em.inputs['Color'].default_value = (*rgb, 1)
        nt.links.new(em.outputs[0], next(x for x in nt.nodes if x.type == 'OUTPUT_MATERIAL').inputs['Surface'])
        mats[name] = mm
        return mm
    black, white, orange = (0.0, 0.0, 0.0), (0.82, 0.82, 0.8), (0.95, 0.32, 0.02)

    def quad(x0, y0, x1, y1, col, z):
        me = bpy.data.meshes.new('q')
        me.from_pydata([(x0, y0, z), (x1, y0, z), (x1, y1, z), (x0, y1, z)], [], [(0, 1, 2, 3)])
        o = bpy.data.objects.new('q', me)
        sc.collection.objects.link(o)
        me.materials.append(mat(str(col), col))
        return o

    def poly(pts, col, z):
        me = bpy.data.meshes.new('p')
        me.from_pydata([(x, y, z) for x, y in pts], [], [tuple(range(len(pts)))])
        o = bpy.data.objects.new('p', me)
        sc.collection.objects.link(o)
        me.materials.append(mat(str(col), col))
        return o

    def text(s, x, y, size, col=black, align='CENTER', sx=0.8):
        size *= 1.3
        cu = bpy.data.curves.new('t', 'FONT')
        cu.body = s
        cu.font = font
        cu.size = size
        cu.align_x = align
        cu.align_y = 'CENTER'
        o = bpy.data.objects.new('t', cu)
        o.location = (x, y, 0.02)
        o.scale = (sx, 1, 1)
        sc.collection.objects.link(o)
        cu.materials.append(mat(str(col), col))
        return o

    def hand(cx, cy, s, col):
        # palm, four fingers, a thumb (an upraised hand)
        quad(cx - 0.32 * s, cy - 0.45 * s, cx + 0.3 * s, cy + 0.1 * s, col, 0.015)
        for k, h in enumerate((0.42, 0.5, 0.48, 0.38)):
            fx = cx - 0.3 * s + k * 0.16 * s
            quad(fx, cy + 0.08 * s, fx + 0.12 * s, cy + (0.1 + h) * s, col, 0.015)
        poly([(cx + 0.28 * s, cy - 0.25 * s), (cx + 0.5 * s, cy + 0.05 * s), (cx + 0.42 * s, cy + 0.12 * s), (cx + 0.25 * s, cy - 0.05 * s)], col, 0.015)

    def walker(cx, cy, s, col):
        bpy.ops.mesh.primitive_circle_add(vertices=16, radius=0.09 * s, fill_type='NGON', location=(cx + 0.04 * s, cy + 0.38 * s, 0.015))
        o = bpy.context.active_object
        o.data.materials.append(mat(str(col), col))
        poly([(cx - 0.05 * s, cy + 0.25 * s), (cx + 0.1 * s, cy + 0.25 * s), (cx + 0.05 * s, cy - 0.05 * s), (cx - 0.08 * s, cy - 0.05 * s)], col, 0.015)
        poly([(cx - 0.08 * s, cy - 0.05 * s), (cx - 0.0 * s, cy - 0.05 * s), (cx - 0.12 * s, cy - 0.45 * s), (cx - 0.2 * s, cy - 0.42 * s)], col, 0.015)
        poly([(cx + 0.0 * s, cy - 0.05 * s), (cx + 0.07 * s, cy - 0.05 * s), (cx + 0.2 * s, cy - 0.45 * s), (cx + 0.12 * s, cy - 0.47 * s)], col, 0.015)
        poly([(cx - 0.02 * s, cy + 0.22 * s), (cx + 0.02 * s, cy + 0.17 * s), (cx - 0.2 * s, cy + 0.02 * s), (cx - 0.24 * s, cy + 0.07 * s)], col, 0.015)
        poly([(cx + 0.06 * s, cy + 0.22 * s), (cx + 0.1 * s, cy + 0.18 * s), (cx + 0.26 * s, cy + 0.04 * s), (cx + 0.22 * s, cy + 0.0 * s)], col, 0.015)

    # the face: black border and panel rules on white
    quad(0, 0, W, H, black, 0.0)
    quad(0.35, 0.35, W - 0.35, H - 0.35, white, 0.005)
    rules = [11.55, 6.75, 4.2]  # panel edges (in, from the bottom)
    for y in rules:
        quad(0.35, y - 0.09, W - 0.35, y + 0.09, black, 0.01)
    # 1: walk
    quad(0.75, 12.35, 2.65, 14.25, black, 0.01)
    walker(1.7, 13.3, 1.6, white)
    text('START CROSSING', 5.75, 13.95, 0.62)
    quad(3.4, 13.55, 8.1, 13.62, black, 0.012)
    text('Watch For', 5.75, 13.05, 0.55)
    text('Vehicles', 5.75, 12.35, 0.55)
    # 2: flashing hand, timer
    quad(0.75, 9.15, 2.55, 10.95, black, 0.01)
    hand(1.65, 10.05, 1.45, orange)
    for k in range(6):
        a = k / 6 * math.tau
        quad(1.65 + math.cos(a) * 1.1 - 0.05, 10.05 + math.sin(a) * 1.1 - 0.05, 1.65 + math.cos(a) * 1.1 + 0.05, 10.05 + math.sin(a) * 1.1 + 0.05, black, 0.012)
    quad(0.85, 7.15, 2.45, 8.75, black, 0.01)
    text('18', 1.65, 7.95, 1.1, orange, sx=0.7)
    text('MAY START', 5.75, 10.9, 0.62)
    quad(3.7, 10.5, 7.8, 10.57, black, 0.012)
    text('Finish Crossing', 5.75, 10.0, 0.55)
    text('If Started', 5.75, 9.3, 0.55)
    text('TIME REMAINING', 5.75, 8.2, 0.62)
    text('To Finish Crossing', 5.75, 7.4, 0.55)
    # 3: steady hand
    quad(0.85, 4.65, 2.45, 6.3, black, 0.01)
    hand(1.65, 5.45, 1.25, orange)
    text("DON'T CROSS", 5.75, 5.45, 0.62)
    # 4: push button, arrow, to cross
    text('PUSH BUTTON', W / 2, 3.35, 0.7)
    ax0, ax1 = (2.0, 6.6) if arrow > 0 else (7.0, 2.4)
    s = 1 if arrow > 0 else -1
    quad(min(ax0, ax1), 2.38, max(ax0, ax1), 2.58, black, 0.012)
    poly([(ax1, 2.08), (ax1 + 0.75 * s, 2.48), (ax1, 2.88)][::s], black, 0.012)
    text('TO CROSS', W / 2, 1.35, 0.65)
    # an orthographic render of the face
    cam = bpy.data.objects.new('signcam', bpy.data.cameras.new('signcam'))
    sc.collection.objects.link(cam)
    cam.data.type = 'ORTHO'
    cam.data.ortho_scale = H
    cam.location = (W / 2, H / 2, 5.0)
    sc.camera = cam
    sc.render.resolution_x, sc.render.resolution_y = px_w, px_h
    sc.render.pixel_aspect_x = 1.0
    sc.render.film_transparent = False
    prev = (sc.cycles.samples, sc.view_settings.view_transform)
    sc.cycles.samples = 32
    sc.view_settings.view_transform = 'Standard'
    sc.render.image_settings.file_format = 'PNG'
    sc.render.image_settings.color_depth = '16'
    path = os.path.join(tdir, f'button-sign-{arrow}.png')
    sc.render.filepath = path
    # the camera frames H vertically; the render's aspect is W:H
    cam.data.sensor_fit = 'VERTICAL'
    bpy.ops.render.render(write_still=True)
    sc.cycles.samples, sc.view_settings.view_transform = prev
    img = bpy.data.images.load(path)
    img.colorspace_settings.name = 'Non-Color'
    a = pixels(img)[..., :3].copy()  # sRGB values (Standard view): linearised below
    bpy.data.images.remove(img)
    for o in [o for o in bpy.data.objects if o not in keep]:
        bpy.data.objects.remove(o, do_unlink=True)
    return np.where(a <= 0.04045, a / 12.92, ((a + 0.055) / 1.055) ** 2.4).astype(np.float32)


# ---------------------------------------------------------------- UV layout of the small pieces

def lay_out_small(o, region, margin=0.004):
    """Project and pack the faces without a laid-out uv into region (u0, v0, u1, v1)."""
    me = o.data
    fixed = me.attributes['fixed_uv']
    select_only([o])
    bpy.ops.object.mode_set(mode='EDIT')
    bm = bmesh.from_edit_mesh(me)
    lay = bm.faces.layers.int['fixed_uv']
    for f in bm.faces:
        f.select = f[lay] == 0
    bmesh.update_edit_mesh(me)
    bpy.ops.uv.smart_project(angle_limit=math.radians(60), island_margin=0.0, area_weight=0.0, scale_to_bounds=False)
    bpy.ops.uv.select_all(action='SELECT')
    bpy.ops.uv.average_islands_scale()
    u0, v0, u1, v1 = region
    aspect = (u1 - u0) / (v1 - v0)
    # pre-stretch v so the square pack comes out at the region's aspect after the squeeze
    bm = bmesh.from_edit_mesh(me)
    uvl = bm.loops.layers.uv['atlas']
    for f in bm.faces:
        if f.select:
            for loop in f.loops:
                loop[uvl].uv.x /= aspect
    bmesh.update_edit_mesh(me)
    bpy.ops.uv.pack_islands(udim_source='CLOSEST_UDIM', rotate=False, scale=True, margin_method='FRACTION', margin=margin)
    bm = bmesh.from_edit_mesh(me)
    uvl = bm.loops.layers.uv['atlas']
    for f in bm.faces:
        if f.select:
            for loop in f.loops:
                uv = loop[uvl].uv
                loop[uvl].uv = (u0 + uv.x * (u1 - u0), v0 + uv.y * (v1 - v0))
    bmesh.update_edit_mesh(me)
    bpy.ops.object.mode_set(mode='OBJECT')


# ---------------------------------------------------------------- bakes and weathering

def new_float_image(name, px):
    img = bpy.data.images.new(name, px, px, alpha=True, float_buffer=True)
    img.colorspace_settings.name = 'Non-Color'
    img.generated_color = (0, 0, 0, 0)
    return img


def pixels(img):
    w, h = img.size
    buf = np.empty(w * h * 4, dtype=np.float32)
    img.pixels.foreach_get(buf)
    return buf.reshape(h, w, 4)  # row 0 = bottom (Blender)


def bake_inputs(o, px, ground=True):
    """Object-space position, normal, AO (with the sidewalk under it), pointiness, kind and piece
    of every texel of o's atlas: a dict of (px, px, k) arrays, Blender row order, and the cover mask."""
    sc = bpy.context.scene
    g = None
    hidden = [x for x in bpy.data.objects if x is not o and not x.hide_render]
    for x in hidden:
        x.hide_render = True
    if ground:
        bpy.ops.mesh.primitive_plane_add(size=8, location=(0, 0, 0))
        g = bpy.context.active_object
    me = o.data
    me.uv_layers['atlas'].active = True
    out = {}
    originals = [s.material for s in o.material_slots]
    piece_attr = me.attributes['piece']
    # a colour attribute with the piece id, for the id bake
    if 'pid' not in me.color_attributes:
        ca = me.color_attributes.new('pid', 'FLOAT_COLOR', 'CORNER')
        for p in me.polygons:
            v = piece_attr.data[p.index].value / 32.0
            for li in p.loop_indices:
                ca.data[li].color = (v, 0, 0, 1)
    for what in ('pos', 'nrm', 'kind', 'point', 'ao'):
        img = new_float_image(f'{o.name}-{what}', px)
        mats = []
        for ki, slot in enumerate(o.material_slots):
            m = bpy.data.materials.new(f'bake-{what}-{ki}')
            m.use_nodes = True
            nt = m.node_tree
            for x in list(nt.nodes):
                if x.type != 'OUTPUT_MATERIAL':
                    nt.nodes.remove(x)
            outn = next(x for x in nt.nodes if x.type == 'OUTPUT_MATERIAL')
            if what != 'ao':
                em = nt.nodes.new('ShaderNodeEmission')
                geo = nt.nodes.new('ShaderNodeNewGeometry')
                if what in ('pos', 'nrm'):
                    enc = nt.nodes.new('ShaderNodeVectorMath')
                    enc.operation = 'MULTIPLY_ADD'
                    enc.inputs[1].default_value = (POS_SCALE,) * 3 if what == 'pos' else (0.5,) * 3
                    enc.inputs[2].default_value = (0.5, 0.5, 0.5)
                    nt.links.new(geo.outputs['Position' if what == 'pos' else 'Normal'], enc.inputs[0])
                    nt.links.new(enc.outputs['Vector'], em.inputs['Color'])
                elif what == 'point':
                    # edges: how far a 6 mm bevel turns the normal (Cycles' Bevel node), any tessellation
                    bev = nt.nodes.new('ShaderNodeBevel')
                    bev.samples = 8
                    bev.inputs['Radius'].default_value = 0.006
                    dp = nt.nodes.new('ShaderNodeVectorMath')
                    dp.operation = 'DOT_PRODUCT'
                    nt.links.new(bev.outputs['Normal'], dp.inputs[0])
                    nt.links.new(geo.outputs['Normal'], dp.inputs[1])
                    inv = nt.nodes.new('ShaderNodeMath')
                    inv.operation = 'SUBTRACT'
                    inv.inputs[0].default_value = 1.0
                    nt.links.new(dp.outputs['Value'], inv.inputs[1])
                    nt.links.new(inv.outputs['Value'], em.inputs['Color'])
                else:
                    attr = nt.nodes.new('ShaderNodeAttribute')
                    attr.attribute_name = 'pid'
                    comb = nt.nodes.new('ShaderNodeCombineColor')
                    sep = nt.nodes.new('ShaderNodeSeparateColor')
                    nt.links.new(attr.outputs['Color'], sep.inputs['Color'])
                    comb.inputs['Red'].default_value = ki / 8.0
                    nt.links.new(sep.outputs['Red'], comb.inputs['Green'])
                    comb.inputs['Blue'].default_value = 1.0
                    nt.links.new(comb.outputs['Color'], em.inputs['Color'])
                nt.links.new(em.outputs['Emission'], outn.inputs['Surface'])
            else:
                b = nt.nodes.new('ShaderNodeBsdfPrincipled')
                nt.links.new(b.outputs[0], outn.inputs['Surface'])
            tgt = nt.nodes.new('ShaderNodeTexImage')
            tgt.image = img
            for x in nt.nodes:
                x.select = False
            tgt.select = True
            nt.nodes.active = tgt
            mats.append(m)
        for slot, m in zip(o.material_slots, mats):
            slot.material = m
        select_only([o])
        if what == 'ao':
            sc.cycles.samples = 48
            sc.world = sc.world or bpy.data.worlds.new('w')
            sc.world.light_settings.distance = 0.35
            bpy.ops.object.bake(type='AO', use_clear=True, margin=8)
        else:
            sc.cycles.samples = 16 if what == 'point' else 1
            bpy.ops.object.bake(type='EMIT', use_clear=True, margin=8)
        out[what] = pixels(img).copy()
        bpy.data.images.remove(img)
        for slot, m in zip(o.material_slots, originals):
            slot.material = m
        for m in mats:
            bpy.data.materials.remove(m)
    if g is not None:
        bpy.data.objects.remove(g, do_unlink=True)
    for x in hidden:
        x.hide_render = False
    out['pos'][..., :3] = (out['pos'][..., :3] - 0.5) / POS_SCALE
    out['nrm'][..., :3] = (out['nrm'][..., :3] - 0.5) / 0.5
    # the cover mask: texels some face wrote (the kind bake's blue is 1 there, 0 outside)
    cover = out['kind'][..., 2] > 0.5
    return out, cover


# value noise in numpy (3D, hashed lattice, smooth interpolation)

def _hash(ix, iy, iz, seed):
    n = (ix * 73856093) ^ (iy * 19349663) ^ (iz * 83492791) ^ (seed * 2654435761)
    n = n & 0xffffffff
    n = (n ^ (n >> 13)) * 1274126177
    n = n & 0xffffffff
    n = n ^ (n >> 16)
    return (n & 0xffff).astype(np.float32) / 65535.0


def vnoise(p, seed):
    pi = np.floor(p).astype(np.int64)
    f = (p - pi).astype(np.float32)
    w = f * f * (3 - 2 * f)
    acc = 0.0
    for dx in (0, 1):
        for dy in (0, 1):
            for dz in (0, 1):
                h = _hash(pi[..., 0] + dx, pi[..., 1] + dy, pi[..., 2] + dz, seed)
                wx = w[..., 0] if dx else 1 - w[..., 0]
                wy = w[..., 1] if dy else 1 - w[..., 1]
                wz = w[..., 2] if dz else 1 - w[..., 2]
                acc = acc + h * wx * wy * wz
    return acc


def fbm(p, seed, octaves=4, lac=2.03, gain=0.5):
    a, s, tot = 0.0, 1.0, 0.0
    amp = 1.0
    for k in range(octaves):
        a = a + amp * vnoise(p * s, seed + k * 101)
        tot += amp
        s *= lac
        amp *= gain
    return a / tot


def smooth(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0, 1)
    return t * t * (3 - 2 * t)


def srgb_to_lin(c):
    c = np.asarray(c, dtype=np.float32) / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def weather(inp, cover, seed, family, c, rust_sources, px, finish='painted'):
    """The albedo (linear), roughness, metalness and height of every texel (Blender row order).
    family: 'pole', 'post' or 'hw'. rust_sources: [(x, y, z, strength, width_m, run_m)] object-space
    points rust bleeds down from."""
    P = inp['pos'][..., :3]
    N = inp['nrm'][..., :3]
    nlen = np.linalg.norm(N, axis=-1, keepdims=True)
    N = N / np.maximum(nlen, 1e-6)
    ao = np.clip(inp['ao'][..., 0], 0, 1)
    kind = np.rint(inp['kind'][..., 0] * 8).astype(int)
    piece = np.rint(inp['kind'][..., 1] * 32).astype(int)
    point = inp['point'][..., 0]
    edge = smooth(0.015, 0.12, point)  # edges (the bevel bake)
    z = P[..., 2]
    up = np.clip(N[..., 2], 0, 1)

    paint = srgb_to_lin(c['paint'])
    zinc = np.array([0.33, 0.335, 0.33], np.float32)
    cast_base = paint * 1.08
    concrete = np.array([0.30, 0.29, 0.27], np.float32)

    alb = np.zeros(P.shape, np.float32)
    rough = np.zeros(z.shape, np.float32)
    metal = np.zeros(z.shape, np.float32)
    height = np.zeros(z.shape, np.float32)

    big = fbm(P * 2.3, seed + 1, 4)
    mid = fbm(P * 9.0, seed + 2, 4)
    fine = fbm(P * 55.0, seed + 3, 3)
    # rain streaks: noise stretched along z
    streak = fbm(P * np.array([30.0, 30.0, 0.7], np.float32), seed + 4, 3)

    is_paint = (kind == K_PAINT) | (kind == K_CAST)
    is_zinc = kind == K_ZINC
    is_conc = kind == K_CONCRETE
    down = np.clip(-N[..., 2], 0, 1)

    # --- paint: the signal colour, chalked by the sun all round (most on the up-facing and upper
    # surfaces), mottled in patches, rain-streaked; the cast fittings a little lighter
    base = np.where((kind == K_CAST)[..., None], cast_base, paint)
    chalk = np.clip(0.48 + 0.35 * up - 0.12 * down + 0.55 * (big - 0.5) * 2, 0, 1)
    if family != 'hw':
        chalk = chalk * (0.75 + 0.25 * smooth(0.3, 3.5, z))
    chalk_col = np.array([0.15, 0.155, 0.145], np.float32)
    col = base * (1 - 0.65 * chalk[..., None]) + chalk_col * (0.65 * chalk[..., None])
    col = col * (0.86 + 0.28 * mid[..., None])
    # rain: pale runs and dark runs down the sides
    runs = (streak - 0.5) * 2
    col = col * (1.0 + 0.3 * np.clip(runs, 0, 1)[..., None] * (1 - down[..., None]))
    col = col * (1.0 - 0.25 * np.clip(-runs, 0, 1)[..., None])
    pr = 0.6 + 0.2 * chalk + 0.08 * (fine - 0.5)

    # chips through the paint to the galvanising: small, at edges, the foot, the bolts; a few anywhere
    foot = smooth(0.9, 0.05, z) if family != 'hw' else np.zeros_like(z)
    chip_n = fbm(P * 110.0, seed + 5, 3)
    chip_p = 0.05 + 0.6 * edge + 0.3 * foot + 0.1 * (piece == P_HANDHOLE) + 0.08 * (piece == P_PLATE) + 0.08 * (piece == P_FLANGE)
    t0 = 1.0 - chip_p * 0.45
    chips = smooth(t0, t0 + 0.02, chip_n) * is_paint
    chip_rim = (smooth(t0 - 0.035, t0, chip_n) - chips).clip(0, 1) * is_paint
    old = smooth(0.45, 0.7, fbm(P * 20.0, seed + 6, 2))  # old chips rusted
    chip_col = zinc * (1 - old[..., None]) + np.array([0.16, 0.085, 0.045], np.float32) * old[..., None]
    col = col * (1 - chips[..., None]) + chip_col * chips[..., None]
    col = col * (1 - 0.35 * chip_rim[..., None])
    pr = pr * (1 - chips) + (0.5 + 0.35 * old) * chips
    pm = 0.0 * (1 - chips) + 0.75 * (1 - old) * chips

    # sticker and tape residue on the shafts at hand height (the city's poles carry plenty)
    if family != 'hw':
        rng = np.random.default_rng(seed + 40)
        theta = np.arctan2(P[..., 1], P[..., 0])
        rad = np.hypot(P[..., 0], P[..., 1])
        torn = fbm(P * 70.0, seed + 41, 3)
        stick = np.zeros_like(z)
        stick_col = np.zeros(P.shape, np.float32)
        for k in range(4 if family == 'pole' else 2):
            tk = rng.uniform(-math.pi, math.pi)
            zk = rng.uniform(1.0, 2.1)
            wk, hk = rng.uniform(0.02, 0.06), rng.uniform(0.025, 0.08)
            arc = np.abs((theta - tk + math.pi) % math.tau - math.pi) * rad
            inside = smooth(wk, wk * 0.8, arc) * smooth(hk, hk * 0.85, np.abs(z - zk))
            left = rng.uniform(0.35, 0.8)  # how much of it is left
            m_k = inside * smooth(1 - left - 0.05, 1 - left + 0.05, torn) * (piece == P_SHAFT)
            tone = np.array([[0.46, 0.46, 0.43], [0.32, 0.33, 0.33], [0.06, 0.06, 0.065], [0.22, 0.25, 0.30]][int(rng.integers(0, 4))], np.float32)
            stick_col = np.where((m_k > stick)[..., None], tone, stick_col)
            stick = np.maximum(stick, m_k)
        col = col * (1 - stick[..., None]) + stick_col * (0.8 + 0.4 * fine[..., None]) * stick[..., None]
        pr = pr * (1 - stick) + 0.85 * stick
        pm = pm * (1 - stick)

    paint_h = -0.6 * chips - 0.25 * chip_rim + 0.08 * (fine - 0.5)
    if finish == 'galvanised':
        # bare hot-dip galvanising gone to its patina: a light dull grey in big soft mottles and
        # darker spangle patches, white-rust bloom, dark and pale rain runs; the coating worn
        # through to rust in pits at the foot; the cast fittings a little darker
        g0 = np.array([0.215, 0.225, 0.232], np.float32)
        gc = np.where((kind == K_CAST)[..., None], g0 * 0.82, g0)
        speck = fbm(P * 200.0, seed + 33, 2)  # at the 512 px maps, the finest the foot column holds
        gc = gc * (0.9 + 0.2 * big[..., None]) * (0.84 + 0.32 * speck[..., None])
        spangle = smooth(0.5, 0.62, fbm(P * 16.0, seed + 30, 3))
        gc = gc * (1 - 0.14 * spangle[..., None])
        bloom = smooth(0.62, 0.78, fbm(P * np.array([22.0, 22.0, 6.0], np.float32), seed + 31, 3))
        gc = gc * (1 - 0.5 * bloom[..., None]) + np.array([0.36, 0.36, 0.34], np.float32) * 0.5 * bloom[..., None]
        gc = gc * (1.0 + 0.12 * np.clip(runs, 0, 1)[..., None]) * (1.0 - 0.4 * np.clip(-runs, 0, 1)[..., None])
        pits = smooth(0.72, 0.76, fbm(P * 140.0, seed + 32, 2)) * smooth(0.35, 0.05, z) if family != 'hw' else np.zeros_like(z)
        gc = gc * (1 - pits[..., None]) + np.array([0.15, 0.075, 0.035], np.float32) * pits[..., None]
        if family != 'hw':
            col = np.where((stick > 0)[..., None], col, gc)  # the stickers stay
            pr = np.where(stick > 0, pr, 0.55 + 0.18 * spangle + 0.1 * bloom + 0.2 * pits)
            pm = np.where(stick > 0, pm, (0.45 - 0.25 * bloom - 0.15 * spangle) * (1 - pits))
        else:
            col, pr, pm = gc, 0.55 + 0.18 * spangle + 0.1 * bloom, 0.45 - 0.25 * bloom - 0.15 * spangle
        paint_h = 0.12 * (fine - 0.5) - 0.3 * pits + 0.1 * spangle
    alb = np.where(is_paint[..., None], col, alb)
    rough = np.where(is_paint, pr, rough)
    metal = np.where(is_paint, pm, metal)
    height = np.where(is_paint, paint_h, height)

    # --- bare zinc (bolts, nuts, washers, straps): dull galvanising, white rust, red rust
    zc = zinc * (0.85 + 0.3 * mid[..., None])
    white = smooth(0.55, 0.75, fbm(P * 30.0, seed + 7, 3))
    zc = zc * (1 - 0.4 * white[..., None]) + np.array([0.42, 0.42, 0.40], np.float32) * 0.4 * white[..., None]
    red = smooth(0.4, 0.7, fbm(P * 24.0, seed + 8, 3)) * (0.5 + 0.5 * (family == 'pole' or family == 'post'))
    zc = zc * (1 - red[..., None]) + np.array([0.19, 0.09, 0.04], np.float32) * red[..., None]
    alb = np.where(is_zinc[..., None], zc, alb)
    rough = np.where(is_zinc, 0.5 + 0.35 * red + 0.1 * white, rough)
    metal = np.where(is_zinc, 0.7 * (1 - red) * (1 - 0.5 * white), metal)
    height = np.where(is_zinc, 0.3 * (fine - 0.5) - 0.2 * red, height)

    # --- a sign's face (the push buttons' R10-3e(CA)): its sheeting white here, the legend
    # composited over it by `weather_family` (this is its weathering: dust, grime, streaks)
    is_sign = kind == K_SIGN
    sc_ = np.array([0.7, 0.7, 0.68], np.float32) * (0.93 + 0.14 * mid[..., None]) * (1.0 - 0.12 * np.clip(-runs, 0, 1)[..., None])
    alb = np.where(is_sign[..., None], sc_, alb)
    rough = np.where(is_sign, 0.55 + 0.1 * fine, rough)
    metal = np.where(is_sign, 0.0, metal)
    height = np.where(is_sign, 0.05 * (fine - 0.5), height)

    # --- concrete foundation: brushed, stained, gum spots
    cc = concrete * 0.82 * (0.85 + 0.3 * mid[..., None]) * (0.9 + 0.2 * fine[..., None])
    gum = smooth(0.82, 0.86, fbm(P * np.array([16.0, 16.0, 1.0], np.float32), seed + 9, 2))
    cc = cc * (1 - 0.7 * gum[..., None])
    # set into the sidewalk: the dark mastic joint round its rim (broken where it has worn), damp
    # stains, rust spread from the anchor nuts
    r_xy = np.hypot(P[..., 0], P[..., 1])
    pad_r = PILE_R if family == 'pole' else 0.24
    joint = smooth(pad_r - 0.03, pad_r - 0.012, r_xy) * (0.55 + 0.45 * smooth(0.3, 0.55, fbm(P * 30.0, seed + 34, 2)))
    cc = cc * (1 - 0.72 * joint[..., None])
    stain = smooth(0.45, 0.75, fbm(P * 7.0, seed + 35, 3))
    cc = cc * (1 - 0.25 * stain[..., None])
    nuts = ANCHOR_CIRCLE / 2 if family == 'pole' else 0.15
    for k in range(4):
        ang = math.pi / 4 + k * math.pi / 2
        d = np.hypot(P[..., 0] - math.cos(ang) * nuts * 1.12, P[..., 1] - math.sin(ang) * nuts * 1.12)
        rs = smooth(0.1, 0.02, d) * (0.4 + 0.6 * fbm(P * 45.0, seed + 36 + k, 2))
        cc = cc * (1 - 0.6 * rs[..., None]) + np.array([0.17, 0.08, 0.035], np.float32) * 0.6 * rs[..., None]
    alb = np.where(is_conc[..., None], cc, alb)
    rough = np.where(is_conc, 0.9, rough)
    height = np.where(is_conc, 0.5 * (fine - 0.5) - 0.3 * gum, height)

    # --- dust on what faces up; grime in the crevices (AO)
    dust = up ** 2 * smooth(0.35, 0.7, big * 0.6 + mid * 0.4)
    dust_col = np.array([0.20, 0.185, 0.16], np.float32)
    alb = alb * (1 - 0.45 * dust[..., None]) + dust_col * 0.45 * dust[..., None]
    rough = np.clip(rough + 0.2 * dust, 0, 1)
    occ = np.clip(1 - ao, 0, 1)
    alb = alb * (1 - 0.55 * occ[..., None])
    # the undersides collect soot and drip lines
    under = down * (0.5 + 0.5 * smooth(0.4, 0.7, fbm(P * np.array([4.0, 4.0, 40.0], np.float32), seed + 12, 3)))
    alb = alb * (1 - 0.15 * under[..., None])

    # --- grime splash up the foot (the pole and posts): dark, brown, broken edge, speckles
    if family != 'hw':
        reach = (0.25 if finish == 'painted' else 0.16) + 0.25 * big
        g = smooth(reach, 0.0, z) * (0.55 + 0.45 * mid)
        speck = smooth(0.66, 0.7, fbm(P * 90.0, seed + 10, 2)) * smooth(0.8, 0.1, z)
        g = np.clip(g + 0.7 * speck, 0, 1) * (~is_conc)
        # dried splash on a dark pole reads lighter: a dusty brown
        grime_col = np.array([0.10, 0.085, 0.064], np.float32) if finish == 'painted' else np.array([0.085, 0.08, 0.072], np.float32)
        alb = alb * (1 - 0.75 * g[..., None]) + grime_col * 0.75 * g[..., None]
        rough = np.clip(rough + 0.15 * g, 0, 1)
        metal = metal * (1 - g)
        # the foundation's ring of dirt round the plate, darker toward the pole
        r_xy = np.hypot(P[..., 0], P[..., 1])
        pad_dirt = smooth(0.45, 0.12, r_xy) * is_conc * (0.5 + 0.5 * mid)
        alb = alb * (1 - 0.45 * pad_dirt[..., None])

    # --- rust bled down from each source: a halo at it, a run below it
    rust_col = np.array([0.20, 0.085, 0.03], np.float32)
    rust = np.zeros(z.shape, np.float32)
    wob = fbm(P * np.array([40.0, 40.0, 3.0], np.float32), seed + 11, 3)
    for (sx, sy, sz, st, w, run) in rust_sources:
        d = np.sqrt((P[..., 0] - sx) ** 2 + (P[..., 1] - sy) ** 2)
        below = sz - z
        halo = smooth(w * 1.8, w * 0.4, np.sqrt(d ** 2 + np.maximum(-below, 0) ** 2 * 4))
        wid = w * (0.6 + 0.8 * wob)
        drip = smooth(wid, wid * 0.2, d) * smooth(run, 0.0, below) * (below > -0.005)
        rust = np.maximum(rust, st * np.clip(halo * 0.8 + drip * (0.4 + 0.6 * wob), 0, 1))
    rust = rust * (~is_zinc)
    alb = alb * (1 - 0.75 * rust[..., None]) + rust_col * 0.75 * rust[..., None]
    rough = np.clip(rough + 0.2 * rust, 0, 1)
    metal = metal * (1 - rust)

    alb = np.clip(alb, 0.004, 0.9)
    rough = np.clip(rough, 0.3, 1.0)
    metal = np.clip(metal, 0.0, 1.0)
    alb[~cover] = alb[cover].mean(axis=0) if cover.any() else 0.05
    return alb, rough, metal, height


def normal_map(height, cover, px, strength):
    """A tangent-space normal map (OpenGL / glTF convention: +Y toward the image's top) from a
    height field in Blender row order (row 0 = bottom: +rows = +v = up)."""
    h = height.copy()
    h[~cover] = 0.0
    dv, du = np.gradient(h)  # rows (v up), cols (u right)
    n = np.stack([-du * strength, -dv * strength, np.ones_like(h)], axis=-1)
    n /= np.linalg.norm(n, axis=-1, keepdims=True)
    return n * 0.5 + 0.5


def box_down(a, f):
    h, w = a.shape[:2]
    return a.reshape(h // f, f, w // f, f, *a.shape[2:]).mean(axis=(1, 3))


def srgb_encode(x):
    x = np.clip(x, 0.0, 1.0)
    return np.where(x <= 0.0031308, x * 12.92, 1.055 * np.power(x, 1 / 2.4) - 0.055)


def save_jpeg(rgb, path, encode, quality=92):
    a = srgb_encode(rgb) if encode else np.clip(rgb, 0, 1)
    h, w = a.shape[:2]
    img = bpy.data.images.new(os.path.basename(path), w, h, alpha=False)
    img.colorspace_settings.name = 'Non-Color'
    rgba = np.concatenate([a, np.ones((h, w, 1))], axis=2).astype(np.float32)
    img.pixels.foreach_set(rgba.ravel())
    img.filepath_raw = path
    img.file_format = 'JPEG'
    img.save(quality=quality)
    bpy.data.images.remove(img)


def body_material(name, tdir, maps):
    """The game material: albedo, normal and metallic-roughness maps (JPEG), uv map `atlas`."""
    m = bpy.data.materials.new('body')
    m.use_nodes = True
    nt = m.node_tree
    b = next(n for n in nt.nodes if n.type == 'BSDF_PRINCIPLED')
    if maps is None:
        b.inputs['Base Color'].default_value = (0.03, 0.033, 0.031, 1)
        b.inputs['Roughness'].default_value = 0.7
        return m
    uvn = nt.nodes.new('ShaderNodeUVMap')
    uvn.uv_map = 'atlas'
    ta = nt.nodes.new('ShaderNodeTexImage')
    ta.image = bpy.data.images.load(maps['albedo'])
    to = nt.nodes.new('ShaderNodeTexImage')
    to.image = bpy.data.images.load(maps['orm'])
    to.image.colorspace_settings.name = 'Non-Color'
    tn = nt.nodes.new('ShaderNodeTexImage')
    tn.image = bpy.data.images.load(maps['normal'])
    tn.image.colorspace_settings.name = 'Non-Color'
    for t in (ta, to, tn):
        nt.links.new(uvn.outputs['UV'], t.inputs['Vector'])
    nt.links.new(ta.outputs['Color'], b.inputs['Base Color'])
    sep = nt.nodes.new('ShaderNodeSeparateColor')
    nt.links.new(to.outputs['Color'], sep.inputs['Color'])
    nt.links.new(sep.outputs['Green'], b.inputs['Roughness'])
    nt.links.new(sep.outputs['Blue'], b.inputs['Metallic'])
    nm = nt.nodes.new('ShaderNodeNormalMap')
    nm.uv_map = 'atlas'
    nt.links.new(tn.outputs['Color'], nm.inputs['Color'])
    nt.links.new(nm.outputs['Normal'], b.inputs['Normal'])
    return m


# ---------------------------------------------------------------- export (as build-street-signals-v1.py)

def canonical_indices(path):
    with open(path, 'rb') as f:
        b = bytearray(f.read())
    jlen = struct.unpack_from('<I', b, 12)[0]
    gj = json.loads(bytes(b[20:20 + jlen]))
    boff = 20 + jlen + 8
    for mesh in gj['meshes']:
        for prim in mesh['primitives']:
            acc = gj['accessors'][prim['indices']]
            view = gj['bufferViews'][acc['bufferView']]
            fmt = {5121: 'B', 5123: 'H', 5125: 'I'}[acc['componentType']]
            start = boff + view.get('byteOffset', 0) + acc.get('byteOffset', 0)
            n = acc['count']
            idx = list(struct.unpack_from(f'<{n}{fmt}', b, start))
            tris = []
            for t in range(0, n, 3):
                a = idx[t:t + 3]
                k = a.index(min(a))
                tris.append(tuple(a[k:] + a[:k]))
            tris.sort()
            struct.pack_into(f'<{n}{fmt}', b, start, *[i for t in tris for i in t])
    with open(path, 'wb') as f:
        f.write(b)


def export(o, path, mat):
    """o (one material, its `atlas` uv) written as a GLB; returns triangles and glTF bounds."""
    e = duplicate(o, os.path.splitext(os.path.basename(path))[0])
    me = e.data
    for uv in [uv for uv in me.uv_layers if uv.name != 'atlas']:
        me.uv_layers.remove(uv)
    for a in [a for a in me.color_attributes]:
        me.color_attributes.remove(a)
    me.materials.clear()
    me.materials.append(mat)
    for p in me.polygons:
        p.material_index = 0
    select_only([e])
    bpy.ops.object.shade_smooth_by_angle(angle=math.radians(35), keep_sharp_edges=False)
    bpy.ops.export_scene.gltf(filepath=path, use_selection=True, export_format='GLB', export_yup=True, export_apply=True,
                              export_image_format='JPEG', export_jpeg_quality=90, export_texcoords=True, export_normals=True,
                              export_tangents=False, export_materials='EXPORT', export_extras=False, export_attributes=False)
    canonical_indices(path)
    me.calc_loop_triangles()
    tris = len(me.loop_triangles)
    mn, mx = bounds([e])
    bpy.data.objects.remove(e, do_unlink=True)
    return tris, ([mn.x, mn.z, -mx.y], [mx.x, mx.z, -mn.y])


def keep_pieces(o, name, keep):
    """A copy of o with only the faces whose piece is in keep."""
    d = duplicate(o, name)
    bm = bmesh.new()
    bm.from_mesh(d.data)
    lay = bm.faces.layers.int['piece']
    bmesh.ops.delete(bm, geom=[f for f in bm.faces if f[lay] not in keep], context='FACES')
    bm.to_mesh(d.data)
    bm.free()
    return d


def rel(p):
    return os.path.relpath(p, ROOT).replace(os.sep, '/')


def r4(v):
    return [round(float(x), 4) for x in v]


# ---------------------------------------------------------------- families

def family_pole(c, a, tdir):
    """The mast pole: its atlas, three levels."""
    H = c['mastH']
    # uv columns: the foot at the highest density, the upper shaft in two columns; the rest packed
    sh = Shaft([(0.0, MAST_CUT_M[0], 0.0, 0.36, 0.0, 1.0), (MAST_CUT_M[0], MAST_CUT_M[1], 0.37, 0.52, 0.0, 1.0), (MAST_CUT_M[1], H, 0.53, 0.68, 0.0, 1.0)], H)
    region = (0.69, 0.0, 1.0, 1.0)
    l0 = mast_pole(c, sh, 0).object('mastPole-l0')
    lay_out_small(l0, region)
    # the levels' own shafts with level 0's pieces they keep (their packed uv)
    lods = [l0, level_from(l0, mast_pole(c, sh, 1).object('mastPole-shaft1'), 'mastPole-l1', {P_PLATE, P_PAD, P_CAP}, 0.5),
            level_from(l0, mast_pole(c, sh, 2).object('mastPole-shaft2'), 'mastPole-l2', set())]
    ra = c['rBase'] + (c['rTop'] - c['rBase']) * c['armH'] / H
    sources = []
    plate_z1 = 0.006 + ANCHOR_D + BASE_PLATE_T
    for k in range(4):
        ang = math.pi / 4 + k * math.pi / 2
        x, y = math.cos(ang) * ANCHOR_CIRCLE / 2, math.sin(ang) * ANCHOR_CIRCLE / 2
        sources.append((x, y, plate_z1 + 0.04, 0.9, 0.022, 0.12))
        # down the plate's edge nearest the nut, onto the foundation
        ex, ey = math.cos(ang) * BASE_PLATE / 2 * 1.3, math.sin(ang) * BASE_PLATE / 2 * 1.3
        sources.append((x * 0.5 + ex * 0.5, y * 0.5 + ey * 0.5, plate_z1, 0.6, 0.03, 0.09))
    # under the arm connection (+X): streaks from the cap screws down the pole's arm side
    for y in (-0.06, 0.0, 0.07):
        sources.append((ra, y, c['armH'] - ARM_PLATE / 2, 0.55, 0.012, 1.6))
    # under the handhole's screws (-X)
    hz = 0.075 + 19 * IN * 0.5 + 0.24
    sources.append((-(c['rBase'] + (c['rTop'] - c['rBase']) * hz / H) - 0.01, 0.0, hz - 0.09, 0.5, 0.008, 0.35))
    sources.append((-(c['rTop']) - 0.01, 0.0, H - 0.05, 0.35, 0.02, 0.6))  # under the cap
    # round 3: the pole cut at its shaft, each level's faces wholly in each span (the same uv)
    spans = {'mastFoot': (-1.0, MAST_CUT_M[0]), 'mastMid': MAST_CUT_M, 'mastTop': (MAST_CUT_M[1], H + 1.0)}
    levels = {'mastPole': lods}
    for name, (z0, z1) in spans.items():
        levels[name] = [z_slice(o, f'{name}-l{k}', z0, z1) for k, o in enumerate(lods)]
    return bake_family(levels, lods[0], c, a, tdir, 'pole', sources)


def z_slice(o, name, z0, z1):
    """A copy of o with only its faces wholly between z0 and z1 (the shaft has rings there)."""
    d = duplicate(o, name)
    d.data.name = name  # its own name: the level's later copies (its export) keep the names they had
    bm = bmesh.new()
    bm.from_mesh(d.data)
    bmesh.ops.delete(bm, geom=[f for f in bm.faces if not all(z0 - 1e-5 <= v.co.z <= z1 + 1e-5 for v in f.verts)], context='FACES')
    bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
    bm.to_mesh(d.data)
    bm.free()
    d.data.update()
    # out of the scene: the family's bake sees only what it saw before the cut (`export` links its own copy)
    bpy.context.scene.collection.objects.unlink(d)
    return d


def family_posts(c, a, tdir):
    """The pedestal and the pedestrian post, one atlas: the post is the pedestal cut short (its
    shaft's top takes the pedestal's top of the shaft column)."""
    H = c['pedestalH']
    sh = Shaft([(0.0, 2.0, 0.0, 0.26, 0.0, 1.0), (2.0, H, 0.27, 0.53, 0.0, 1.0)], H)
    region = (0.54, 0.0, 1.0, 1.0)
    ped0 = pedestal(c, sh, H, 0).object('pedestalPole-l0')
    lay_out_small(ped0, region)
    ped1 = level_from(ped0, pedestal(c, sh, H, 1).object('pedestalPole-shaft1'), 'pedestalPole-l1', {P_PAD, P_CAP, P_BASE}, 0.3)
    ped2 = level_from(ped0, pedestal(c, sh, H, 2).object('pedestalPole-shaft2'), 'pedestalPole-l2', set())
    posts = [cut_short(o, H, c['postH'], f'pedPost-l{k}') for k, o in enumerate((ped0, ped1, ped2))]
    sources = []
    for k in range(4):
        ang = math.pi / 4 + k * math.pi / 2
        sources.append((math.cos(ang) * 0.15, math.sin(ang) * 0.15, 0.006 + 0.04 + 0.75 * IN, 0.8, 0.02, 0.06))
    sources.append((-PIPE_R - 0.01, 0.0, 0.36 - 0.05, 0.45, 0.006, 0.25))
    sources.append((PIPE_R, 0.0, H - 0.05, 0.3, 0.02, 0.5))
    return bake_family({'pedestalPole': [ped0, ped1, ped2], 'pedPost': posts}, ped0, c, a, tdir, 'post', sources)


def cut_short(o, H, h, name):
    """The pedestal (height H) as a post of height h: everything over the shaft's lower column
    moved down by H - h (the shaft's upper segment is squeezed, its uv kept)."""
    d = duplicate(o, name)
    dz = H - h
    bm = bmesh.new()
    bm.from_mesh(d.data)
    lay = bm.faces.layers.int['piece']
    shaft_v = {v for f in bm.faces if f[lay] == P_SHAFT for v in f.verts}
    top = H - 0.04  # the shaft's top (pedestal())
    for v in bm.verts:
        if v.co.z <= 2.0 + 1e-4:
            continue
        if v in shaft_v:
            v.co.z = 2.0 + (v.co.z - 2.0) * (top - dz - 2.0) / (top - 2.0)
        else:
            v.co.z -= dz
    bm.to_mesh(d.data)
    bm.free()
    d.data.update()
    return d


def decimate(o, ratio):
    """o reduced to `ratio` of its triangles by edge collapse (its uv kept: a far level samples
    level 0's atlas)."""
    select_only([o])
    m = o.modifiers.new('far', 'DECIMATE')
    m.decimate_type = 'COLLAPSE'
    m.ratio = ratio
    m.use_collapse_triangulate = True
    bpy.ops.object.modifier_apply(modifier=m.name)
    return o


def level_from(l0, shaft_obj, name, keep, ratio=1.0):
    """A level: its own shaft (and the pieces built with it whose uv is laid out, i.e. the shaft)
    with level 0's pieces in keep (their packed uv), reduced to `ratio` of their triangles."""
    bm = bmesh.new()
    bm.from_mesh(shaft_obj.data)
    lay = bm.faces.layers.int['fixed_uv']
    bmesh.ops.delete(bm, geom=[f for f in bm.faces if f[lay] == 0], context='FACES')
    bm.to_mesh(shaft_obj.data)
    bm.free()
    parts = [shaft_obj]
    if keep:
        k = keep_pieces(l0, name + '-keep', keep)
        parts.append(decimate(k, ratio) if ratio < 1.0 else k)
    return join(parts, name)


def family_hardware(c, a, tdir):
    """The arm parts and the mounts, one atlas (per finish)."""
    fr = head_frames()
    H = c['mastH']
    r_at = lambda z: c['rBase'] + (c['rTop'] - c['rBase']) * z / H
    tube_rect = (0.0, 0.0, 1.0, 0.16)
    region = (0.0, 0.17, 1.0, 1.0)
    side_h = c['pedestalHeadC'] + 0.15  # a far-side head on a mast pole (signals.rs `head_layout`)
    parts = {
        'armRoot': lambda k: arm_root(c, min(k, 1)),
        'armTip': lambda k: arm_tip(k),
        'mastMount0': lambda k: mast_mount(MOUNT_ARM_RADII[0], k),
        'mastMount1': lambda k: mast_mount(MOUNT_ARM_RADII[1], k),
        'mastMount2': lambda k: mast_mount(MOUNT_ARM_RADII[2], k),
        'sideMountPipe': lambda k: side_mount(PIPE_R, c['lateral'], k, fr['headHalf']),
        'sideMountMast': lambda k: side_mount(r_at(side_h), c['lateral'], k, fr['headHalf']),
        'frontMountPipe': lambda k: side_mount(PIPE_R, FRONT_REACH, k, fr['headHalf']),
        'frontMountMast': lambda k: side_mount(r_at(c['pedestalHeadC']), FRONT_REACH, k, fr['headHalf']),
        'pedMountPipe': lambda k: ped_mount(PIPE_R, fr['pedHalf'], k),
        'pedMountMast': lambda k: ped_mount(r_at(c['pedC']), fr['pedHalf'], k),
        'pedReach': lambda k: ped_reach(fr['pedHalf'], k),
    }
    # level 0 of every part, side by side (offset in y so their bakes do not occlude each
    # other), packed together; level 1 the level-0 pieces it keeps
    objs, offs = [], {}
    for i, (name, f) in enumerate(parts.items()):
        o = f(0).object(name + '-l0')
        off = Vector((0, 3.0 * (i + 1), 0))
        o.data.transform(Matrix.Translation(off))
        objs.append(o)
        offs[name] = off
    tube0 = arm_tube(0, tube_rect).object('armTube-l0')
    tube0.data.transform(Matrix.Diagonal((ARM_NOMINAL_M, 1, 1, 1)))
    allp = join(objs, 'hw-small')
    lay_out_small(allp, region)
    atlas = join([duplicate(allp, 'hw-atlas-small'), duplicate(tube0, 'hw-atlas-tube')], 'hw-atlas')
    sources = []
    ra = r_at(c['armH'])
    x2 = ra + 0.035 + POLE_PLATE_T + ARM_PLATE_T
    o = offs['armRoot']
    for k in range(4):
        ang = math.pi / 4 + k * math.pi / 2
        sources.append((x2 + 0.03, o.y + math.cos(ang) * CAP_SCREW_CIRCLE / 2, math.sin(ang) * CAP_SCREW_CIRCLE / 2, 0.75, 0.015, 0.25))
    for name in parts:
        if name.startswith('mastMount'):
            sources.append((offs[name].x, offs[name].y, offs[name].z - 0.1, 0.4, 0.03, 0.12))
    # the tube: unit length again
    tube_lods = [tube0] + [arm_tube(k, tube_rect).object(f'armTube-l{k}') for k in (1, 2)]
    tube_lods[0].data.transform(Matrix.Diagonal((1 / ARM_NOMINAL_M, 1, 1, 1)))
    levels = {'armTube': tube_lods}
    for name in parts:
        # this part's faces: those within its offset band
        l0 = isolate(allp, name, offs[name])
        l1 = keep_pieces(l0, name + '-x1', {P_GUSSET, P_PLATE, P_FLANGE, P_ENDCAP, P_STRAP, P_SADDLE, P_PIPE, P_BAND})
        if name != 'pedReach':  # (two plain pipes already)
            decimate(l1, 0.4)
        for x in (l0, l1):
            x.data.transform(Matrix.Translation(-offs[name]))
        levels[name] = [l0, l1]
    return bake_family(levels, atlas, c, a, tdir, 'hw', sources)


def isolate(allp, name, off):
    """The faces of allp whose centre is within 1.4 m (y) of off: one part's level 0."""
    d = duplicate(allp, name + '-iso')
    bm = bmesh.new()
    bm.from_mesh(d.data)
    bmesh.ops.delete(bm, geom=[f for f in bm.faces if abs(f.calc_center_median().y - off.y) > 1.4], context='FACES')
    bm.to_mesh(d.data)
    bm.free()
    return d


def suffix(k, finish='painted'):
    return FINISH_SUFFIX[finish] if k == 0 else f'-lod{k}'


def bake_family(levels, baked, c, a, tdir, family, sources, fam=None, one_finish=()):
    """Bake `baked` (the family's level 0, every part laid out on one atlas) once, weather it in
    every finish, export each part's level 0 per finish and its far levels (no maps) once.
    levels: part name -> its level objects (a level the same object as the one before it shares
    its file; a part of one level is that file at every level). `one_finish`: parts drawn the
    same in every finish (their painted files listed for each). Returns finish -> part name ->
    [(path, tris, bounds)]."""
    fam = fam or family
    maps = weather_family(fam, baked, c, a, tdir, family, sources)
    plain = body_material(fam + '-plain', tdir, None)
    out = {f: {} for f in FINISHES}
    far = {}
    for name, objs in levels.items():
        far[name] = []
        for k, o in enumerate(objs[1:], start=1):
            if o is objs[k - 1] and far[name]:
                far[name].append(far[name][-1])
                continue
            path = os.path.join(a['out'], f'{name}-v1{suffix(k)}.glb')
            far[name].append((path, *export(o, path, plain)))
    for f in FINISHES:
        mat = body_material(f'{fam}-{f}', tdir, maps[f])
        for name, objs in levels.items():
            if name in one_finish and f != FINISHES[0]:
                out[f][name] = out[FINISHES[0]][name]
                continue
            path = os.path.join(a['out'], f'{name}-v1{suffix(0, f)}.glb')
            l0 = (path, *export(objs[0], path, mat))
            out[f][name] = [l0] + (far[name] if far[name] else [l0, l0])
    return out


# Per family, the drawn sign faces composited into its atlas: [(uv rect, linear RGB image)].
SIGN_FACES = {}


def resize_img(a, h, w):
    """Bilinear resize of an (H, W, 3) image to (h, w, 3)."""
    H, W = a.shape[:2]
    ys = np.clip((np.arange(h) + 0.5) * H / h - 0.5, 0, H - 1)
    xs = np.clip((np.arange(w) + 0.5) * W / w - 0.5, 0, W - 1)
    y0, x0 = np.floor(ys).astype(int), np.floor(xs).astype(int)
    y1, x1 = np.minimum(y0 + 1, H - 1), np.minimum(x0 + 1, W - 1)
    fy, fx = (ys - y0)[:, None, None], (xs - x0)[None, :, None]
    t = a[y0][:, x0] * (1 - fx) + a[y0][:, x1] * fx
    b = a[y1][:, x0] * (1 - fx) + a[y1][:, x1] * fx
    return (t * (1 - fy) + b * fy).astype(np.float32)


def family_round2(c, a, tdir):
    """Round 2's parts, one atlas of their own (round 1's atlases unchanged): the rising arms
    (one per ES-7E row, three levels, on the tube's strip), hangers for their thicker sections,
    the Street Name signs' brackets, the push buttons with their R10-3e(CA) signs."""
    fr = head_frames()
    H = c['mastH']
    r_at = lambda z: c['rBase'] + (c['rTop'] - c['rBase']) * z / H
    tube_rect = (0.0, 0.0, 1.0, 0.12)
    sign_p, sign_n = (0.0, 0.13, 0.25, 0.55), (0.26, 0.13, 0.51, 0.55)
    region = (0.52, 0.13, 1.0, 1.0)
    classes = arm_classes()
    parts = {
        'signBracket': lambda k: sign_bracket(k),
        'pushButtonPipeP': lambda k: push_button(PIPE_R, 1, k, sign_p),
        'pushButtonPipeN': lambda k: push_button(PIPE_R, -1, k, sign_n),
    }
    objs, offs = [], {}
    for i, (name, f) in enumerate(parts.items()):
        o = f(0).object(name + '-l0')
        off = Vector((0, 3.0 * (i + 1), 0))
        o.data.transform(Matrix.Translation(off))
        objs.append(o)
        offs[name] = off
    tube0 = arm_tube(0, tube_rect).object('hw2-tube')
    tube0.data.transform(Matrix.Diagonal((ARM_NOMINAL_M, 1, 1, 1)))
    allp = join(objs, 'hw2-small')
    lay_out_small(allp, region)
    atlas = join([duplicate(allp, 'hw2-atlas-small'), duplicate(tube0, 'hw2-atlas-tube')], 'hw2-atlas')
    sources = []
    for name in parts:
        if name.startswith('mastMount'):
            sources.append((offs[name].x, offs[name].y, offs[name].z - 0.1, 0.4, 0.03, 0.12))
        if name.startswith('pushButton'):
            sources.append((offs[name].x + PIPE_R + 0.03, offs[name].y, 0.07, 0.35, 0.03, 0.25))
    # the sign faces (the P parts' arrow is the part's +Z: the viewer's left; N's the right)
    px = a['bake']
    fh = int(0.42 * px)
    fw = int(round(fh * BUTTON_SIGN_M[0] / BUTTON_SIGN_M[1]))
    SIGN_FACES['hw2'] = [(sign_p, render_button_sign(-1, fw, fh, tdir)), (sign_n, render_button_sign(1, fw, fh, tdir))]
    # (the props library's 512 meshes: each rising arm two levels, its far level its middle one;
    # the bracket one level; the bracket and the buttons the same files in both finishes)
    levels = {}
    for k, cl in enumerate(classes):
        l1 = arm_rise(cl, 1, tube_rect).object(f'armRise{k}-l1')
        levels[f'armRise{k}'] = [arm_rise(cl, 0, tube_rect).object(f'armRise{k}-l0'), l1, l1]
    for name in parts:
        l0 = isolate(allp, name, offs[name])
        if name == 'signBracket':
            l0.data.transform(Matrix.Translation(-offs[name]))
            levels[name] = [l0]
            continue
        l1 = keep_pieces(l0, name + '-x1', {P_GUSSET, P_PLATE, P_FLANGE, P_ENDCAP, P_STRAP, P_SADDLE, P_PIPE, P_BAND, P_SIGN, P_HANDHOLE})
        if name != 'signBracket':
            decimate(l1, 0.4)
        for x in (l0, l1):
            x.data.transform(Matrix.Translation(-offs[name]))
        levels[name] = [l0, l1]
    # round 4: the level arms at their class's diameter and taper (no rise), on the same strip;
    # out of the scene, so the family's bake sees only what it saw before them
    for k, cl in enumerate(classes):
        flat = dict(cl, rise=0.0, p=1.0)
        l0 = arm_rise(flat, 0, tube_rect).object(f'armLevel{k}-l0')
        l1 = arm_rise(flat, 1, tube_rect).object(f'armLevel{k}-l1')
        for o in (l0, l1):
            bpy.context.scene.collection.objects.unlink(o)
        levels[f'armLevel{k}'] = [l0, l1, l1]
    return bake_family(levels, atlas, c, a, tdir, 'hw', sources, fam='hw2', one_finish={'signBracket', 'pushButtonPipeP', 'pushButtonPipeN'})



# ---------------------------------------------------------------- round 5: the corner hardware
#
# The controller cabinet (`cabinet`, the city's MSF60 on its pad), the No. 5 traffic pull box
# beside a signal pole (`pullBox`) and the stiffener channel across an arm sign's back
# (`signChannel`): two families of their own (their own atlases), built after round 1 to 4's,
# which are left as they were.

P_CAB, P_DOOR, P_HOOD, P_VENT, P_HANDLE, P_HINGE, P_RIM, P_LID, P_CHANNEL = range(22, 31)
MSF60 = SOURCES['mccainMsf60']['read']
CAB_W, CAB_D, CAB_H = MSF60['widthIn'] * IN, MSF60['depthIn'] * IN, MSF60['heightIn'] * IN
CAB_PAD_T, CAB_PAD_OUT = 0.076, 0.06  # modellingValues cabinetPad
N30 = SOURCES['christyN30']['read']
PB_OUT = (N30['outsideLengthIn'] * IN, N30['outsideWidthIn'] * IN)
PB_OPEN = (N30['openingLengthIn'] * IN, N30['openingWidthIn'] * IN)
PB_LID_TOP, PB_RIM_TOP = 0.004, 0.006  # its lid and wall just proud of the sidewalk (it is drawn on it)
PB_LEGEND = 'TRAFFIC SIGNAL'  # modellingValues pullBoxLegend
CHANNEL = (1.5 * IN, 0.75 * IN, 0.09 * IN)  # web, flange, thickness: modellingValues signChannel
PB_LID_RECT = (0.0, 0.47, 1.0, 1.0)  # the lid's top on the pull box atlas (its legend drawn there)


def ring_box(m, rings, piece, kind, cap0=False, cap1=True, ch=0.0):
    """A box-like solid through rectangular rings [(x0, y0, x1, y1, z)], their corners cut by ch."""
    bm = m.bm
    loops = []
    for (x0, y0, x1, y1, z) in rings:
        if ch > 0:
            pts = [(x1 - ch, y0), (x1, y0 + ch), (x1, y1 - ch), (x1 - ch, y1), (x0 + ch, y1), (x0, y1 - ch), (x0, y0 + ch), (x0 + ch, y0)]
        else:
            pts = [(x1, y0), (x1, y1), (x0, y1), (x0, y0)]
        loops.append([bm.verts.new((x, y, z)) for x, y in pts])
    n = len(loops[0])
    for lo, hi in zip(loops, loops[1:]):
        for i in range(n):
            j = (i + 1) % n
            m.face([lo[i], lo[j], hi[j], hi[i]], piece, kind)
    if cap0:
        m.face(list(reversed(loops[0])), piece, kind)
    if cap1:
        m.face(loops[-1], piece, kind)
    return loops


def louvre(m, x0, x1, y, z0, z1, slats, axis='y', sign=-1.0):
    """A louvred vent on a face (its plane at y, or x for axis 'x'): wedge slats sloping out and down."""
    bm = m.bm
    pitch = (z1 - z0) / slats
    out = 0.011
    for k in range(slats):
        zt = z1 - k * pitch
        zb = zt - pitch * 0.8
        def P(u, d, z):  # u along the face, d out of it
            return bm.verts.new((u, y + sign * d, z) if axis == 'y' else (y + sign * d, u, z))
        a0, a1 = P(x0, 0.0, zt), P(x1, 0.0, zt)
        b0, b1 = P(x0, out, zb), P(x1, out, zb)
        c0, c1 = P(x0, 0.0, zb), P(x1, 0.0, zb)
        o = Vector((0, sign, 0)) if axis == 'y' else Vector((sign, 0, 0))
        along = Vector((1, 0, 0)) if axis == 'y' else Vector((0, 1, 0))
        oface(m, [a0, a1, b1, b0], P_VENT, K_ZINC, o + Vector((0, 0, 1)))
        oface(m, [b0, b1, c1, c0], P_VENT, K_ZINC, Vector((0, 0, -1)))
        oface(m, [a0, b0, c0], P_VENT, K_ZINC, -along)
        oface(m, [a1, c1, b1], P_VENT, K_ZINC, along)


def oface(m, verts, piece, kind, want):
    """A face wound so its normal looks along `want`."""
    n = (verts[1].co - verts[0].co).cross(verts[2].co - verts[1].co)
    return m.face(verts if n.dot(want) >= 0 else list(reversed(verts)), piece, kind)


def cabinet(lod):
    """The MSF60 controller cabinet on its pad: origin at the pad's foot centre on the sidewalk,
    its doors on -Y (glTF +Z), its width along X."""
    m = Mesh()
    W, D, H = CAB_W, CAB_D, CAB_H
    zb = CAB_PAD_T
    zt = zb + H - 0.045
    po = CAB_PAD_OUT
    e = 0.012 if lod == 0 else 0.0
    ring_box(m, [(-W / 2 - po, -D / 2 - po, W / 2 + po, D / 2 + po, -0.05), (-W / 2 - po, -D / 2 - po, W / 2 + po, D / 2 + po, zb - e)]
             + ([(-W / 2 - po + e, -D / 2 - po + e, W / 2 + po - e, D / 2 + po - e, zb)] if e else []), P_PAD, K_CONCRETE)
    # the shell, its base flange, the hood with its drip lip
    ring_box(m, [(-W / 2, -D / 2, W / 2, D / 2, zb + 0.03), (-W / 2, -D / 2, W / 2, D / 2, zt)], P_CAB, K_ZINC, cap1=False, ch=0.006)
    ring_box(m, [(-W / 2 - 0.01, -D / 2 - 0.01, W / 2 + 0.01, D / 2 + 0.01, zb), (-W / 2 - 0.01, -D / 2 - 0.01, W / 2 + 0.01, D / 2 + 0.01, zb + 0.035)],
             P_CAB, K_ZINC, ch=0.004)
    hx0, hx1, hy0, hy1 = -W / 2 - 0.025, W / 2 + 0.025, -D / 2 - 0.035, D / 2 + 0.015
    ring_box(m, [(hx0, hy0, hx1, hy1, zt), (hx0, hy0, hx1, hy1, zt + 0.03), (hx0 + 0.01, hy0 + 0.01, hx1 - 0.01, hy1 - 0.01, zt + 0.045)],
             P_HOOD, K_ZINC, cap0=True)
    if lod == 2:
        return m
    box(m, (hx0, hy0 - 0.003, zt - 0.018), (hx1, hy0, zt), P_HOOD, K_ZINC)
    # the two doors, raised on the front, the seam between them
    yd = -D / 2
    dz0, dz1 = zb + 0.065, zt - 0.035
    for x0, x1 in ((-W / 2 + 0.022, -0.007), (0.007, W / 2 - 0.022)):
        box(m, (x0, yd - 0.004, dz0), (x1, yd + 0.002, dz1), P_DOOR, K_ZINC)
    if lod == 1:
        return m
    yf = yd - 0.004
    for sgn in (-1, 1):
        xc = sgn * 0.058
        # the latch handle on two stand-offs, the lock over it
        box(m, (xc - 0.011, yf - 0.032, zb + 0.86), (xc + 0.011, yf - 0.022, zb + 1.04), P_HANDLE, K_ZINC)
        for zz in (zb + 0.875, zb + 1.025):
            box(m, (xc - 0.008, yf - 0.022, zz - 0.008), (xc + 0.008, yf, zz + 0.008), P_HANDLE, K_ZINC)
        tube(m, 'y', (xc, yf, zb + 1.10), (xc, yf - 0.012, zb + 1.10), 0.014, 0.013, 12, P_HANDLE, K_ZINC, cap1=True)
        # three hinges on the door's outer edge
        xh = sgn * (W / 2 - 0.016)
        for zz in (zb + 0.22, zb + 0.74, zb + 1.26):
            tube(m, 'z', (xh, yf - 0.004, zz), (xh, yf - 0.004, zz + 0.075), 0.009, 0.009, 8, P_HINGE, K_ZINC, cap0=True, cap1=True)
        # the louvred intake at the door's foot
        xm = sgn * (W / 4 + 0.008)
        louvre(m, xm - 0.15, xm + 0.15, yf, zb + 0.12, zb + 0.27, 6)
    # the police panel door on the right door, its key lock
    box(m, (0.13, yf - 0.003, zb + 1.16), (0.29, yf + 0.001, zb + 1.34), P_DOOR, K_ZINC)
    tube(m, 'y', (0.21, yf - 0.003, zb + 1.30), (0.21, yf - 0.011, zb + 1.30), 0.011, 0.01, 10, P_HANDLE, K_ZINC, cap1=True)
    # the exhaust vents high on both sides
    for sgn in (-1, 1):
        louvre(m, -0.12, 0.12, sgn * W / 2, zt - 0.2, zt - 0.08, 4, axis='x', sign=sgn)
    return m


def pull_box(lod):
    """The No. 5 traffic pull box in the sidewalk: its precast lid and the box's top wall round it,
    origin at the lid's centre on the sidewalk, its length along X. The lid's top carries the
    legend on its own uv rect (PB_LID_RECT)."""
    m = Mesh()
    bm = m.bm
    L, Wd = PB_OUT
    ol, ow = PB_OPEN
    gap = 0.004
    ll, lw = ol - 2 * gap, ow - 2 * gap
    # the wall's top: a frame between its outside and the opening, its outside down into the sidewalk
    out = [bm.verts.new(p) for p in ((L / 2, -Wd / 2, PB_RIM_TOP), (L / 2, Wd / 2, PB_RIM_TOP), (-L / 2, Wd / 2, PB_RIM_TOP), (-L / 2, -Wd / 2, PB_RIM_TOP))]
    inn = [bm.verts.new(p) for p in ((ol / 2, -ow / 2, PB_RIM_TOP), (ol / 2, ow / 2, PB_RIM_TOP), (-ol / 2, ow / 2, PB_RIM_TOP), (-ol / 2, -ow / 2, PB_RIM_TOP))]
    low = [bm.verts.new((v.co.x, v.co.y, -0.03)) for v in out]
    deep = [bm.verts.new((v.co.x, v.co.y, PB_LID_TOP - 0.03)) for v in inn]
    for i in range(4):
        j = (i + 1) % 4
        m.face([out[i], out[j], inn[j], inn[i]], P_RIM, K_CONCRETE)
        m.face([low[i], low[j], out[j], out[i]], P_RIM, K_CONCRETE)
        m.face([inn[i], inn[j], deep[j], deep[i]], P_RIM, K_CONCRETE)
    # the lid: its sides into the seam, its top on the legend's rect
    lo = [bm.verts.new(p) for p in ((ll / 2, -lw / 2, PB_LID_TOP - 0.03), (ll / 2, lw / 2, PB_LID_TOP - 0.03), (-ll / 2, lw / 2, PB_LID_TOP - 0.03), (-ll / 2, -lw / 2, PB_LID_TOP - 0.03))]
    hi = [bm.verts.new((v.co.x, v.co.y, PB_LID_TOP)) for v in lo]
    for i in range(4):
        j = (i + 1) % 4
        m.face([lo[i], lo[j], hi[j], hi[i]], P_LID, K_CONCRETE)
    u0, v0, u1, v1 = PB_LID_RECT
    uv = lambda v: ((v.co.x / ll + 0.5) * (u1 - u0) + u0, (v.co.y / lw + 0.5) * (v1 - v0) + v0)
    m.face(hi, P_LID, K_CONCRETE, [uv(v) for v in hi])
    return m


def sign_channel(lod):
    """A stiffener channel across a sign's back, unit length along X (the game scales it to the
    sign's width): its flanges on the back (y = 0, the back's plane; glTF -Z behind the face),
    its web 3/4 in out. Origin at the middle of its contact line."""
    m = Mesh()
    web, fl, t = CHANNEL
    box(m, (-0.5, fl - t, -web / 2), (0.5, fl, web / 2), P_CHANNEL, K_ZINC)
    for s_ in (-1, 1):
        z0, z1 = (web / 2 - t, web / 2) if s_ > 0 else (-web / 2, -web / 2 + t)
        box(m, (-0.5, 0.0, z0), (0.5, fl - t, z1), P_CHANNEL, K_ZINC)
    return m


def text_mask(lines, w_in, h_in, px_w, px_h, tdir, name):
    """Black legend lines [(text, x_in, y_in, cap_in)] on white over a w x h in plane, rendered
    orthographically: the legend's coverage (1 on the letters), rows from the bottom."""
    keep = set(bpy.data.objects)
    sc = bpy.context.scene
    font = bpy.data.fonts.load(OVERPASS['bold'])

    def mat(rgb):
        mm = bpy.data.materials.new('mask')
        mm.use_nodes = True
        nt = mm.node_tree
        for x in list(nt.nodes):
            if x.type != 'OUTPUT_MATERIAL':
                nt.nodes.remove(x)
        em = nt.nodes.new('ShaderNodeEmission')
        em.inputs['Color'].default_value = (*rgb, 1)
        nt.links.new(em.outputs[0], next(x for x in nt.nodes if x.type == 'OUTPUT_MATERIAL').inputs['Surface'])
        return mm
    me = bpy.data.meshes.new('q')
    me.from_pydata([(-2, -2, 0), (w_in + 2, -2, 0), (w_in + 2, h_in + 2, 0), (-2, h_in + 2, 0)], [], [(0, 1, 2, 3)])
    o = bpy.data.objects.new('q', me)
    sc.collection.objects.link(o)
    me.materials.append(mat((1.0, 1.0, 1.0)))
    black = mat((0.0, 0.0, 0.0))
    for (txt, x, y, cap) in lines:
        cu = bpy.data.curves.new('t', 'FONT')
        cu.body = txt
        cu.font = font
        cu.size = cap * 1.3
        cu.align_x = 'CENTER'
        cu.align_y = 'CENTER'
        t = bpy.data.objects.new('t', cu)
        t.location = (x, y, 0.02)
        sc.collection.objects.link(t)
        cu.materials.append(black)
    cam = bpy.data.objects.new('maskcam', bpy.data.cameras.new('maskcam'))
    sc.collection.objects.link(cam)
    cam.data.type = 'ORTHO'
    cam.data.ortho_scale = h_in
    cam.data.sensor_fit = 'VERTICAL'
    cam.location = (w_in / 2, h_in / 2, 5.0)
    sc.camera = cam
    sc.render.resolution_x, sc.render.resolution_y = px_w, px_h
    sc.render.pixel_aspect_x = 1.0
    sc.render.film_transparent = False
    prev = (sc.cycles.samples, sc.view_settings.view_transform)
    sc.cycles.samples = 16
    sc.view_settings.view_transform = 'Standard'
    sc.render.image_settings.file_format = 'PNG'
    sc.render.image_settings.color_depth = '16'
    path = os.path.join(tdir, f'{name}.png')
    sc.render.filepath = path
    bpy.ops.render.render(write_still=True)
    sc.cycles.samples, sc.view_settings.view_transform = prev
    img = bpy.data.images.load(path)
    img.colorspace_settings.name = 'Non-Color'
    a = pixels(img)[..., 0].copy()
    bpy.data.images.remove(img)
    for x in [x for x in bpy.data.objects if x not in keep]:
        bpy.data.objects.remove(x, do_unlink=True)
    return (1.0 - np.clip(a, 0, 1)).astype(np.float32)


# Per round-5 family, its legend masks: [(uv rect, coverage image, what)].
R5_MASKS = {}


def family_cabinet(c, a, tdir):
    """The controller cabinet on its pad: one atlas of its own, three levels."""
    l0 = cabinet(0).object('cabinet-l0')
    lay_out_small(l0, (0.0, 0.0, 1.0, 1.0), margin=0.006)
    atlas = duplicate(l0, 'cab-atlas')
    l1 = keep_pieces(l0, 'cabinet-l1', {P_PAD, P_CAB, P_HOOD, P_DOOR})
    l2 = keep_pieces(l0, 'cabinet-l2', {P_PAD, P_CAB, P_HOOD})
    for o in (l1, l2):
        bpy.context.scene.collection.objects.unlink(o)
    levels = {'cabinet': [l0, l1, l2]}
    return bake_family(levels, atlas, c, a, tdir, 'cab', [], fam='cab', one_finish={'cabinet'})


def family_pull_box(c, a, tdir):
    """The pull box and the sign's stiffener channel: one atlas of their own, the lid's legend on
    its rect."""
    pb = pull_box(0).object('pullBox-l0')
    ch = sign_channel(0).object('signChannel-l0')
    offs = {'pullBox': Vector((0, 0.0, 0)), 'signChannel': Vector((0, 3.0, 1.5))}
    ch.data.transform(Matrix.Translation(offs['signChannel']))
    allp = join([pb, ch], 'pbx-small')
    lay_out_small(allp, (0.0, 0.0, 1.0, PB_LID_RECT[1] - 0.01), margin=0.01)
    atlas = duplicate(allp, 'pbx-atlas')
    levels = {}
    for name in ('pullBox', 'signChannel'):
        o = isolate(allp, name, offs[name])
        o.data.transform(Matrix.Translation(-offs[name]))
        levels[name] = [o]
    # the lid's legend (1 1/4 in capitals along it) and its two lifting slots, on the lid's rect
    px = a['bake']
    u0, v0, u1, v1 = PB_LID_RECT
    w_px, h_px = int(round((u1 - u0) * px)), int(round((v1 - v0) * px))
    ll, lw = (PB_OPEN[0] - 0.008) / IN, (PB_OPEN[1] - 0.008) / IN
    legend = text_mask([(PB_LEGEND, ll / 2, lw / 2, 1.25)], ll, lw, w_px, h_px, tdir, 'pull-box-legend')
    xs = np.linspace(0, ll, w_px)[None, :]
    ys = np.linspace(0, lw, h_px)[:, None]
    slots = np.zeros((h_px, w_px), np.float32)
    for sx in (1.6, ll - 1.6):
        slots = np.maximum(slots, smooth(0.3, 0.2, np.abs(xs - sx)) * smooth(1.05, 0.95, np.abs(ys - lw / 2)))
    R5_MASKS['pbx'] = [(PB_LID_RECT, legend, 'legend'), (PB_LID_RECT, slots, 'slots')]
    return bake_family(levels, atlas, c, a, tdir, 'pbx', [], fam='pbx', one_finish={'pullBox', 'signChannel'})


def weather_r5(inp, cover, seed, family, px):
    """Round 5's surfaces: the cabinet's dull mill-finish aluminium (rain runs under the hood, a
    splash at the foot, hand grime by the handles, graffiti painted out in mismatched patches,
    sticker residue), its concrete pad; the pull box's concrete wall and lid (the legend and
    slots cast in, dirt in them); the channel's aluminium. Same returns as `weather`."""
    P = inp['pos'][..., :3]
    N = inp['nrm'][..., :3]
    N = N / np.maximum(np.linalg.norm(N, axis=-1, keepdims=True), 1e-6)
    ao = np.clip(inp['ao'][..., 0], 0, 1)
    kind = np.rint(inp['kind'][..., 0] * 8).astype(int)
    piece = np.rint(inp['kind'][..., 1] * 32).astype(int)
    edge = smooth(0.015, 0.12, inp['point'][..., 0])
    z = P[..., 2]
    up = np.clip(N[..., 2], 0, 1)
    down = np.clip(-N[..., 2], 0, 1)
    side = 1 - up - down
    big = fbm(P * 2.3, seed + 1, 4)
    mid = fbm(P * 9.0, seed + 2, 4)
    fine = fbm(P * 55.0, seed + 3, 3)
    streak = fbm(P * np.array([34.0, 34.0, 0.8], np.float32), seed + 4, 3)
    runs = (streak - 0.5) * 2
    is_alu = kind == K_ZINC
    is_conc = kind == K_CONCRETE

    # --- aluminium: mill finish gone dull and oxidised, rain runs down the sides
    ac = np.array([0.30, 0.305, 0.30], np.float32) * (0.88 + 0.24 * big[..., None]) * (0.93 + 0.14 * mid[..., None])
    oxid = smooth(0.52, 0.74, fbm(P * 11.0, seed + 50, 3))
    ac = ac * (1 - 0.2 * oxid[..., None]) + np.array([0.36, 0.36, 0.345], np.float32) * 0.2 * oxid[..., None] * smooth(0.6, 0.8, fine)[..., None]
    ac = ac * (1 - 0.32 * np.clip(-runs, 0, 1)[..., None] * side[..., None]) * (1 + 0.08 * np.clip(runs, 0, 1)[..., None])
    ar = 0.52 + 0.14 * oxid + 0.08 * (fine - 0.5)
    am = 0.55 * (1 - 0.35 * oxid)
    ah = 0.08 * (fine - 0.5) + 0.05 * oxid
    if family == 'cab':
        # hand grime round the handles, on the doors
        front = smooth(0.6, 0.9, -N[..., 1])
        for sgn in (-1, 1):
            d = np.hypot((P[..., 0] - sgn * 0.058) / 0.09, (z - (CAB_PAD_T + 0.98)) / 0.16)
            hg = smooth(1.0, 0.2, d) * front * (0.5 + 0.5 * mid)
            ac = ac * (1 - 0.35 * hg[..., None])
            ar = ar - 0.1 * hg
        # graffiti painted out: patches of flat paint of the wrong grey, their roller edges ragged
        rng = np.random.default_rng(seed + 60)
        paint = np.zeros_like(z)
        pcol = np.zeros(P.shape, np.float32)
        tones = [(0.205, 0.205, 0.195), (0.24, 0.225, 0.19), (0.16, 0.17, 0.17), (0.215, 0.225, 0.2), (0.27, 0.27, 0.26)]
        faces = [(np.array([0, -1, 0]), 0), (np.array([0, 1, 0]), 0), (np.array([1, 0, 0]), 1), (np.array([-1, 0, 0]), 1)]
        rag = fbm(P * 26.0, seed + 61, 3)
        for k in range(5):
            nrm, ax = faces[int(rng.integers(0, 4))] if k > 0 else faces[1]
            onf = smooth(0.8, 0.95, N @ nrm.astype(np.float32))
            span = CAB_W / 2 if ax == 0 else CAB_D / 2
            uc = rng.uniform(-span * 0.6, span * 0.6)
            hw = rng.uniform(0.12, 0.42) * (1.0 if ax == 0 else 0.6)
            zc = rng.uniform(0.45, 1.25)
            hh = rng.uniform(0.12, 0.35)
            u = P[..., ax]
            inside = smooth(hw + 0.02, hw - 0.02, np.abs(u - uc) + 0.03 * (rag - 0.5)) * smooth(hh + 0.02, hh - 0.02, np.abs(z - zc) + 0.03 * (rag - 0.5))
            m_k = inside * onf * (piece != P_HANDLE) * (piece != P_HINGE) * is_alu
            pcol = np.where((m_k > paint)[..., None], np.array(tones[int(rng.integers(0, len(tones)))], np.float32), pcol)
            paint = np.maximum(paint, m_k)
        pcol = pcol * (0.94 + 0.12 * mid[..., None])
        ac = ac * (1 - paint[..., None]) + pcol * paint[..., None]
        ar = ar * (1 - paint) + 0.82 * paint
        am = am * (1 - paint)
        ah = ah + 0.06 * paint
        # sticker residue (white, yellow, black, blue labels, mostly torn off) at hand height
        torn = fbm(P * 70.0, seed + 62, 3)
        stick = np.zeros_like(z)
        scol = np.zeros(P.shape, np.float32)
        for k in range(7):
            nrm, ax = faces[int(rng.integers(0, 4))]
            onf = smooth(0.85, 0.95, N @ nrm.astype(np.float32))
            span = CAB_W / 2 if ax == 0 else CAB_D / 2
            uc = rng.uniform(-span * 0.8, span * 0.8)
            zc = rng.uniform(0.9, 1.55)
            hw, hh = rng.uniform(0.02, 0.07), rng.uniform(0.02, 0.05)
            inside = smooth(hw, hw * 0.85, np.abs(P[..., ax] - uc)) * smooth(hh, hh * 0.85, np.abs(z - zc))
            left = rng.uniform(0.3, 0.85)
            m_k = inside * onf * smooth(1 - left - 0.05, 1 - left + 0.05, torn) * is_alu
            tone = [(0.6, 0.6, 0.57), (0.55, 0.45, 0.08), (0.03, 0.03, 0.035), (0.08, 0.13, 0.3), (0.45, 0.43, 0.4)][int(rng.integers(0, 5))]
            scol = np.where((m_k > stick)[..., None], np.array(tone, np.float32), scol)
            stick = np.maximum(stick, m_k)
        ac = ac * (1 - stick[..., None]) + scol * (0.8 + 0.3 * fine[..., None]) * stick[..., None]
        ar = ar * (1 - stick) + 0.7 * stick
        am = am * (1 - stick)
        # the splash at the foot: dried mud and soot up to a third of a metre
        reach = CAB_PAD_T + 0.18 + 0.16 * big
        g = smooth(reach, CAB_PAD_T, z) * (0.55 + 0.45 * mid) * is_alu
        ac = ac * (1 - 0.6 * g[..., None]) + np.array([0.075, 0.068, 0.058], np.float32) * 0.6 * g[..., None]
        ar = ar + 0.15 * g
        am = am * (1 - 0.7 * g)
    alb = np.where(is_alu[..., None], ac, 0.0)
    rough = np.where(is_alu, ar, 0.0)
    metal = np.where(is_alu, am, 0.0)
    height = np.where(is_alu, ah, 0.0)

    # --- concrete: the cabinet's pad and the pull box (its wall lighter, its lid darker and
    # smoother, the legend and slots cast in with the dirt in them)
    base = np.where((piece == P_LID)[..., None], np.array([0.205, 0.2, 0.19], np.float32), np.array([0.30, 0.29, 0.27], np.float32))
    cc = base * (0.85 + 0.3 * mid[..., None]) * (0.9 + 0.2 * fine[..., None])
    gum = smooth(0.82, 0.86, fbm(P * np.array([16.0, 16.0, 1.0], np.float32), seed + 9, 2))
    cc = cc * (1 - 0.6 * gum[..., None])
    stain = smooth(0.45, 0.75, fbm(P * 7.0, seed + 35, 3))
    cc = cc * (1 - 0.25 * stain[..., None])
    cc = cc * (1 - 0.25 * edge[..., None])  # worn, dirty arrises
    cr = np.where(piece == P_LID, 0.78, 0.9) + 0.05 * (fine - 0.5)
    ch = 0.4 * (fine - 0.5) - 0.3 * gum
    for rect, mask, what in R5_MASKS.get(family, []):
        c0, c1 = int(round(rect[0] * px)), int(round(rect[2] * px))
        r0, r1 = int(round(rect[1] * px)), int(round(rect[3] * px))
        mk = np.zeros(z.shape, np.float32)
        mk[r0:r1, c0:c1] = resize_img(np.repeat(mask[..., None], 3, -1), r1 - r0, c1 - c0)[..., 0]
        mk = mk * (piece == P_LID) * smooth(0.8, 0.95, up)
        if what == 'legend':
            cc = cc * (1 - 0.5 * mk[..., None])
            ch = ch - 0.9 * mk
            cr = cr + 0.08 * mk
        else:
            cc = cc * (1 - 0.85 * mk[..., None])
            ch = ch - 1.6 * mk
    alb = np.where(is_conc[..., None], cc, alb)
    rough = np.where(is_conc, cr, rough)
    height = np.where(is_conc, ch, height)

    # --- dust on what faces up, grime in the crevices
    dust = up ** 2 * smooth(0.35, 0.7, big * 0.6 + mid * 0.4)
    alb = alb * (1 - 0.4 * dust[..., None]) + np.array([0.20, 0.185, 0.16], np.float32) * 0.4 * dust[..., None]
    rough = np.clip(rough + 0.2 * dust, 0, 1)
    occ = np.clip(1 - ao, 0, 1)
    alb = alb * (1 - 0.55 * occ[..., None])
    alb = np.clip(alb, 0.004, 0.9)
    rough = np.clip(rough, 0.3, 1.0)
    metal = np.clip(metal, 0.0, 1.0)
    alb[~cover] = alb[cover].mean(axis=0) if cover.any() else 0.05
    return alb, rough, metal, height


def weather_family(fam, o, c, a, tdir, family, sources):
    """o's surfaces baked once and weathered in every finish: finish -> its maps (JPEG paths)."""
    px = a['bake']
    inp, cover = bake_inputs(o, px, ground=(family != 'hw'))
    out = {}
    for finish in FINISHES:
        if family in ('cab', 'pbx'):
            alb, rough, metal, height = weather_r5(inp, cover, a['seed'] + len(fam), family, px)
        else:
            alb, rough, metal, height = weather(inp, cover, a['seed'] + len(fam), family, c, sources, px, finish)
        for rect, face in SIGN_FACES.get(fam, []):
            # the drawn face over the weathered sheeting (rows from the bottom, as the uv's v)
            c0, c1 = int(round(rect[0] * px)), int(round(rect[2] * px))
            r0, r1 = int(round(rect[1] * px)), int(round(rect[3] * px))
            dirt = alb[r0:r1, c0:c1] / np.array([0.7, 0.7, 0.68], np.float32)
            f = resize_img(face, r1 - r0, c1 - c0)
            sign = np.rint(inp['kind'][r0:r1, c0:c1, 0] * 8).astype(int) == K_SIGN
            alb[r0:r1, c0:c1] = np.where(sign[..., None], np.clip(f * dirt, 0.004, 0.9), alb[r0:r1, c0:c1])
        nrm = normal_map(height, cover, px, strength=2.0)
        f = px // 512
        alb5, rough5, metal5, nrm5 = box_down(alb, f), box_down(rough, f), box_down(metal, f), box_down(nrm, f)
        nrm5 = (nrm5 * 2 - 1)
        nrm5 /= np.linalg.norm(nrm5, axis=-1, keepdims=True)
        nrm5 = nrm5 * 0.5 + 0.5
        orm = np.stack([np.ones_like(rough5), rough5, metal5], axis=-1)
        maps = {k: os.path.join(tdir, f'{fam}-{finish}-{k}.jpg') for k in ('albedo', 'orm', 'normal')}
        save_jpeg(alb5, maps['albedo'], encode=True)
        save_jpeg(orm, maps['orm'], encode=False)
        save_jpeg(nrm5, maps['normal'], encode=False, quality=95)
        out[finish] = maps
    return out


# ---------------------------------------------------------------- look renders

def look_renders(a, c, files):
    """Cycles renders of the built files (with the heads) under the HDRI: the whole assembly, the
    foot, the arm's root."""
    reset()
    sc = bpy.context.scene
    sc.cycles.samples = a['samples']
    sc.render.resolution_x, sc.render.resolution_y = 900, 1200
    w = bpy.data.worlds.new('w')
    sc.world = w
    w.use_nodes = True
    if a['hdri']:
        env = w.node_tree.nodes.new('ShaderNodeTexEnvironment')
        env.image = bpy.data.images.load(a['hdri'])
        bg = next(n for n in w.node_tree.nodes if n.type == 'BACKGROUND')
        w.node_tree.links.new(env.outputs['Color'], bg.inputs['Color'])
    sc.view_settings.view_transform = 'AgX'

    def place(path, loc, yaw=0.0, sx=1.0):
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=path)
        new = [o for o in bpy.data.objects if o not in before]
        for o in new:
            if o.parent is None:
                o.matrix_world = Matrix.Translation(loc) @ Matrix.Rotation(yaw, 4, 'Z') @ Matrix.Diagonal((sx, 1, 1, 1)) @ o.matrix_world
        return new
    out = a['out']
    sig = os.path.join(ROOT, 'public', 'assets', 'street-signals')
    L = 7.0
    place(os.path.join(out, 'mastPole-v1.glb'), Vector((0, 0, 0)))
    place(os.path.join(out, 'armRoot-v1.glb'), Vector((0, 0, c['armH'])))
    place(os.path.join(out, 'armTube-v1.glb'), Vector((0, 0, c['armH'])), sx=L)
    place(os.path.join(out, 'armTip-v1.glb'), Vector((L, 0, c['armH'])))
    for x in (3.0, 6.2):
        r = ARM_R0 + (ARM_R1 - ARM_R0) * x / L
        k = min(range(3), key=lambda i: abs(MOUNT_ARM_RADII[i] - r))
        place(os.path.join(out, f'mastMount{k}-v1.glb'), Vector((x, 0, c['armH'])))
        place(os.path.join(sig, 'signalHead-v1.glb'), Vector((x, 0, c['mastHeadC'])))
    # a far-side head beside the mast pole (toward +X, facing -Y)
    place(os.path.join(out, 'sideMountMast-v1.glb'), Vector((0, 0, c['pedestalHeadC'] + 0.15)))
    place(os.path.join(sig, 'signalHead-v1.glb'), Vector((c['lateral'], 0, c['pedestalHeadC'] + 0.15)))
    # a pedestal with its head, a pedestrian post with its head
    place(os.path.join(out, 'pedestalPole-v1.glb'), Vector((2.2, -2.0, 0)), yaw=math.pi)
    place(os.path.join(out, 'sideMountPipe-v1.glb'), Vector((2.2, -2.0, c['pedestalHeadC'])), yaw=0.0)
    place(os.path.join(sig, 'signalHead-v1.glb'), Vector((2.2 + c['lateral'], -2.0, c['pedestalHeadC'])))
    place(os.path.join(out, 'pedPost-v1.glb'), Vector((-1.6, -1.2, 0)), yaw=2.0)
    place(os.path.join(out, 'pedMountPipe-v1.glb'), Vector((-1.6, -1.2, c['pedC'])), yaw=-math.pi / 2)
    place(os.path.join(sig, 'pedHead-v1.glb'), Vector((-1.6, -1.2 - c['pedStandoff'], c['pedC'])))
    # the galvanised finish: a second mast pole and arm behind, a pedestal
    gs = FINISH_SUFFIX['galvanised']
    place(os.path.join(out, f'mastPole-v1{gs}.glb'), Vector((-3.0, 4.0, 0)))
    place(os.path.join(out, f'armRoot-v1{gs}.glb'), Vector((-3.0, 4.0, c['armH'])))
    place(os.path.join(out, f'armTube-v1{gs}.glb'), Vector((-3.0, 4.0, c['armH'])), sx=5.0)
    place(os.path.join(out, f'armTip-v1{gs}.glb'), Vector((2.0, 4.0, c['armH'])))
    place(os.path.join(out, f'mastMount1-v1{gs}.glb'), Vector((0.5, 4.0, c['armH'])))
    place(os.path.join(sig, 'signalHead-v1.glb'), Vector((0.5, 4.0, c['mastHeadC'])))
    place(os.path.join(out, f'pedestalPole-v1{gs}.glb'), Vector((4.2, -1.0, 0)), yaw=math.pi)
    place(os.path.join(out, f'sideMountPipe-v1{gs}.glb'), Vector((4.2, -1.0, c['pedestalHeadC'])))
    place(os.path.join(sig, 'signalHead-v1.glb'), Vector((4.2 + c['lateral'], -1.0, c['pedestalHeadC'])))
    # round 2: a rising arm (class 3, 9.1 m) on a third pole, push buttons on the pedestal and the mast
    if os.path.exists(os.path.join(out, 'armRise3-v1.glb')):
        cl = arm_classes()[3]
        y0 = c['armH'] - 0.9
        place(os.path.join(out, 'mastPole-v1.glb'), Vector((-6.0, 9.0, 0)), yaw=-math.pi / 2)
        place(os.path.join(out, 'armRoot-v1.glb'), Vector((-6.0, 9.0, y0 + cl['rise'] * (1 - (1 - 0.245 / cl['length']) ** cl['p']))), yaw=-math.pi / 2)
        place(os.path.join(out, 'armRise3-v1.glb'), Vector((-6.0, 9.0, y0)), yaw=-math.pi / 2)
        place(os.path.join(out, 'armTip-v1.glb'), Vector((-6.0, 9.0 - cl['length'], y0 + cl['rise'])), yaw=-math.pi / 2)
        place(os.path.join(out, 'pushButtonPipeN-v1.glb'), Vector((2.2, -2.0, BUTTON_HEIGHT)), yaw=-math.pi / 2)
        # on the mast pole: the pipe's unit set out to the mast's face (dims pushButtonMastOffsetM)
        off = c['rBase'] + (c['rTop'] - c['rBase']) * BUTTON_HEIGHT / c['mastH'] - PIPE_R
        place(os.path.join(out, 'pushButtonPipeP-v1.glb'), Vector((0, -off, BUTTON_HEIGHT)), yaw=-math.pi / 2)
    bpy.ops.mesh.primitive_plane_add(size=30, location=(0, 0, -0.001))
    g = bpy.context.active_object
    gm = bpy.data.materials.new('sidewalk')
    gm.use_nodes = True
    gb = next(n for n in gm.node_tree.nodes if n.type == 'BSDF_PRINCIPLED')
    gb.inputs['Base Color'].default_value = (0.32, 0.31, 0.29, 1)
    gb.inputs['Roughness'].default_value = 0.9
    g.data.materials.append(gm)
    cam = bpy.data.objects.new('cam', bpy.data.cameras.new('cam'))
    sc.collection.objects.link(cam)
    sc.camera = cam
    shots = {
        'all': ((6.5, -11.0, 2.0), (2.5, 0, 3.6), 40),
        'foot': ((1.0, -1.6, 0.75), (0, 0, 0.25), 45),
        'foot_galv': ((-2.0, 2.4, 0.75), (-3.0, 4.0, 0.25), 45),
        'button': ((0.55, -1.0, 1.25), (0.0, -0.1, 1.2), 35),
        'rise': ((3.0, -6.0, 2.0), (-6.0, 4.0, 5.6), 50),
        'joint': ((1.3, -2.2, 5.8), (0.3, 0, 6.35), 40),
        'posts': ((0.5, -6.5, 1.7), (0.5, -1.5, 2.0), 45),
    }
    os.makedirs(a['renders'], exist_ok=True)
    for name, (eye, at, lens_deg) in shots.items():
        cam.location = eye
        d = Vector(at) - Vector(eye)
        cam.rotation_euler = d.to_track_quat('-Z', 'Y').to_euler()
        cam.data.angle = math.radians(lens_deg)
        sc.render.filepath = os.path.join(a['renders'], f'{name}.png')
        bpy.ops.render.render(write_still=True)


# ---------------------------------------------------------------- main

def main():
    a = args()
    c = js_consts()
    os.makedirs(a['out'], exist_ok=True)
    global BUTTON_SIGN_M, BUTTON_HEIGHT
    std = {e['id']: e for e in json.load(open(STANDARDS))['entries']}
    s_ = std['ped-push-button-sign']['metricValue']
    BUTTON_SIGN_M = (s_['r10_3eWidth'], s_['r10_3eHeight'])
    BUTTON_HEIGHT = std['ped-push-button-location']['metricValue']['heightApprox']
    if a['only'] == 'renders':
        look_renders(a, c, None)
        return
    tdir = os.path.join(a['out'], '.textures')
    os.makedirs(tdir, exist_ok=True)
    files = {f: {} for f in FINISHES}
    if not os.path.exists(OVERPASS['bold']):
        raise SystemExit(f'{OVERPASS["bold"]} missing (python3 scripts/setup-assets.py fetches the sign art)')
    SOURCES['overpass']['sha256'] = sha256(OVERPASS['bold'])
    for fam in (family_pole, family_posts, family_hardware, family_round2, family_cabinet, family_pull_box):
        reset()
        for f, parts in fam(c, a, tdir).items():
            files[f].update(parts)
    for f in os.listdir(tdir):
        os.remove(os.path.join(tdir, f))
    os.rmdir(tdir)
    finishes = {}
    for finish in FINISHES:
        parts = {}
        for name, levels in files[finish].items():
            rows = []
            for path, tris, (mn, mx) in levels:
                rows.append({'path': rel(path), 'sha256': sha256(path), 'bytes': os.path.getsize(path), 'triangles': tris,
                             'boundsM': {'min': r4(mn), 'max': r4(mx)}, 'dimensionsM': r4([mx[i] - mn[i] for i in range(3)])})
            while len(rows) < 3:
                rows.append(rows[-1])
            parts[name] = {'lods': rows}
        finishes[finish] = parts
    fr = head_frames()
    prov = {
        'version': 'signal-poles-v1',
        'note': 'San Francisco traffic-signal poles, mast arms and head mounts, modelled in code from the dimensions of Caltrans RSP ES-7C '
                '(sources) and the game\'s signal-hardware constants (props-js.json.gz), with modelling values where no sheet was read '
                '(modellingValues). No third-party geometry or image is used: the maps are baked and weathered by the script. Frames: glTF Y up; '
                'a pole\'s origin at its foot on its axis, its arm side +X (the handhole at -X); an arm part\'s origin on the arm axis, +X along the arm '
                '(armTube unit length, scaled to the arm); a mount\'s origin on the pole (or arm) axis at the head\'s centre (the arm), +X toward the head. '
                'Every part in two finishes (finishes): painted and galvanised; their level 0 files differ (the maps), the far levels are shared.',
        'script': rel(SCRIPT), 'scriptSha256': sha256(SCRIPT), 'blender': bpy.app.version_string,
        'sources': SOURCES,
        'modellingValues': MODELLING,
        'dims': {
            'mastPoleHeightM': c['mastH'], 'pedestalHeightM': c['pedestalH'], 'postHeightM': c['postH'],
            'armRootRadiusM': ARM_R0, 'armTipRadiusM': ARM_R1, 'mountArmRadiiM': list(MOUNT_ARM_RADII),
            'sideReachM': c['lateral'], 'frontReachM': FRONT_REACH, 'pipeRadiusM': round(PIPE_R, 5),
            'headHalfM': fr['headHalf'], 'pedHalfM': fr['pedHalf'], 'pedBackM': fr['pedBack'],
            'armClasses': [{'lengthM': r4([k['length']])[0], 'riseM': r4([k['rise']])[0], 'angleDeg': k['angleDeg'], 'odAtPoleM': r4([k['od']])[0],
                            'rootRadiusM': r4([k['r0']])[0], 'tipRadiusM': r4([k['r1']])[0], 'exponent': r4([k['p']])[0]} for k in arm_classes()],
            'armFlangeXM': r4([c['rBase'] + (c['rTop'] - c['rBase']) * c['armH'] / c['mastH'] + 0.035 + POLE_PLATE_T + ARM_PLATE_T])[0],
            'pushButtonSignM': list(BUTTON_SIGN_M), 'pushButtonHeightM': BUTTON_HEIGHT,
            # a mast pole's button: the pedestal pipe's part, set out to the mast's face at the button's height
            'pushButtonMastOffsetM': r4([c['rBase'] + (c['rTop'] - c['rBase']) * BUTTON_HEIGHT / c['mastH'] - PIPE_R])[0],
            # the mast pole's cuts (mastFoot below the first, mastTop above the second)
            'mastCutM': list(MAST_CUT_M),
            # round 5: the cabinet (width, depth, height over its pad; the pad's height and its margin), the pull
            # box (its outside: length, width), the sign channel (web height, depth out of the back)
            'cabinetM': r4([CAB_W, CAB_D, CAB_H]), 'cabinetPadM': r4([CAB_PAD_T, CAB_PAD_OUT]),
            'pullBoxM': r4(list(PB_OUT)), 'signChannelM': r4([CHANNEL[0], CHANNEL[1]]),
        },
        'finishes': finishes,
    }
    with open(os.path.join(a['out'], 'signal-poles-v1.provenance.json'), 'w') as f:
        json.dump(prov, f, indent=2)
        f.write('\n')
    print(json.dumps({k: [l['triangles'] for l in v['lods']] for k, v in finishes['painted'].items()}))
    if a['renders']:
        look_renders(a, c, files)


if __name__ == '__main__':
    main()
