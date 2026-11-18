# Alignment review shots: close-ups of every panel junction, closed and open.
#   Blender -b project-sedan-v1.blend -P render_closeups.py [-- <out_dir>]
# Prefix c_ = closed pose, o_ = everything open. Compare against renders/close/ after any rebuild.
import bpy, sys, os, math
from mathutils import Vector
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import build_sedan
argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
OUT = argv[0] if argv else os.path.join(os.path.dirname(os.path.abspath(__file__)), 'renders', 'close')
os.makedirs(OUT, exist_ok=True)
sc = bpy.context.scene; cam = bpy.data.objects['Cam']
def shot(name, loc, tgt, lens=70):
    cam.data.lens = lens; cam.location = loc
    cam.rotation_euler = (Vector(tgt) - cam.location).to_track_quat('-Z', 'Y').to_euler()
    sc.render.filepath = f'{OUT}/{name}.png'; bpy.ops.render.render(write_still=True)
build_sedan.pose(False)
shot('c_apillar', (-2.6, -1.9, 1.5), (-0.85, -0.45, 1.0), 85)        # mirror, A-pillar, windscreen edge, front door frame
shot('c_bpillar', (-2.8, 0.6, 1.3), (-0.88, 0.44, 0.95), 85)         # door shut line at B-pillar, handles, belt seam
shot('c_cpillar', (-2.6, 2.4, 1.5), (-0.85, 1.35, 1.0), 85)          # rear door frame / C-pillar / quarter glass
shot('c_hoodgap', (-2.2, -3.2, 1.4), (-0.6, -1.6, 0.8), 70)          # hood edge vs fender, headlamp cluster
shot('c_trunkgap', (1.9, 3.4, 1.5), (0.55, 2.0, 0.85), 70)           # trunk edge, taillamp, fuel door
shot('c_arch', (-2.4, -1.5, 0.6), (-0.9, -1.47, 0.45), 70)           # wheel arch lip, rocker
shot('c_nose', (0.0, -4.2, 0.55), (0.0, -2.3, 0.5), 60)              # grille, plate, fog lamps, valance
shot('c_tail', (0.0, 4.2, 0.7), (0.0, 2.3, 0.6), 60)                 # tail, exhaust, diffuser
build_sedan.pose(True)
shot('o_window_in', (0.35, 0.1, 1.05), (-0.85, -0.1, 0.7), 24)       # rolled-down driver window seen from inside
shot('o_glovebox', (0.15, 0.25, 0.95), (0.40, -0.45, 0.70), 30)      # open glovebox
shot('o_doorjamb', (-2.2, -0.2, 1.0), (-0.85, 0.0, 0.7), 50)         # open front door: aperture, card, hinge side
shot('o_hoodbay', (-1.3, -3.0, 1.8), (0.0, -1.5, 0.6), 45)           # engine bay under open hood
shot('o_trunkbay', (1.3, 3.4, 1.8), (0.0, 2.0, 0.6), 45)             # trunk bay under open lid
shot('o_fuel', (2.4, 1.2, 1.1), (0.9, 1.65, 0.78), 85)               # open fuel door
build_sedan.pose(False)
