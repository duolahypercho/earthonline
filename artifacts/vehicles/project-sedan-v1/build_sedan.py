import bpy, bmesh, math, sys, os, importlib
from mathutils import Vector, Matrix, Euler
SP = os.path.dirname(os.path.abspath(__file__))
if SP not in sys.path: sys.path.insert(0, SP)
import sedan_lib as S
importlib.reload(S)
from sedan_lib import *

def build():
    clear(); M = materials(); setup_world()
    BP, GP = body_profile(), green_profile()

    # ---- body + greenhouse as one envelope surface (see sedan_lib.envelope_loft) ----
    body = envelope_loft('Body', BP, GP, M['paint'], segs=96, step=0.015)
    # plain body-only shell for clipping trim parts to the flanks/nose/tail
    shell0 = loft('ShellRef', BP, M['paint'], segs=96, step=0.015); shell0.hide_render = True; shell0.hide_viewport = True

    # ---- panels first, from the pristine envelope ----
    # The exact boolean solver classifies inside/outside reliably on the untouched loft; once arches
    # and the cabin void are in, piece extraction becomes order-dependent. So hood, trunk, doors and
    # fuel door are cut out now, and later cuts are applied to the body and to the panels they touch.
    panels = extract_panels(body, shell0, M, BP)

    # ---- wheel wells (body, hood, rear doors) ----
    ARCH_R = R + 0.055; DEPTH = 0.30
    for ay in (FRONT_Y, REAR_Y):
        for sx in (-1, 1):
            targets = [body, panels['hood']] if ay < 0 else [body, panels['doors'][('R', 'L' if sx < 0 else 'R')]]
            for t in targets:
                boolean_apply(t, prim_cyl('ArchCutter', ARCH_R, DEPTH + 0.1, (sx * (W / 2 - DEPTH / 2 + 0.05), ay, R), 'X', segs=96))

    # ---- hollow the cabin (body and door panels) ----
    # The body's square shoulder (see sedan_lib.body_ntop) buries the greenhouse's rounded base, so the
    # belt line is a single envelope surface instead of a glasshouse overhanging a round shoulder.
    for t in [body] + list(panels['doors'].values()):
        boolean_apply(t, loft('CabinVoid', GP, M['interior'], segs=64, step=0.02, expand=-0.025, y_range=(-0.72, 1.80)))   # 2.5 cm pillars/roof
        boolean_apply(t, prim_box('Cut', (1.64, 2.55, 0.70), (0, 0.50, 0.65)))                                           # floor to belt, out to the door skins

    def clip_to(ob, ref, scale=(1.0, 1.0, 1.0), dz=0.0):
        sh = ref.copy(); sh.data = ref.data.copy(); link(sh); sh.modifiers.clear(); sh.hide_render = False; sh.hide_viewport = False
        sh.scale = scale; sh.location.z = dz
        boolean_apply(ob, sh, 'INTERSECT')

    # ---- lamp clusters (front + rear corners) ----
    # Each cluster is one empty (`Headlamp_L`, `Taillamp_R`, ...) holding the
    # housing, the clear cover and one mesh per light function, each with its
    # own material so the runtime can switch them independently:
    #   VehicleLowBeam / VehicleHighBeam / VehicleDRL / VehicleIndicator (front)
    #   VehicleTaillamp / VehicleBrakeLamp / VehicleIndicator / VehicleReverseLamp (rear)
    lamp_groups = {}
    def recess_and_lamp(name, x, y, z, size, yaw, lamp_mat):
        front = y < 0
        for sx in (-1, 1):
            tag = ('L' if sx < 0 else 'R')
            gname = f'{name}_{tag}'
            group = bpy.data.objects.new(gname, None); group.location = (sx * x, y, z); link(group); group.empty_display_size = 0.05
            members = []
            cutter = prim_box('Cut', size, (sx * x, y, z), rot=(0, 0, sx * yaw))
            boolean_apply(body, cutter)
            housing = prim_box(gname + '_Housing', (size[0] * 0.995, size[1] * 0.995, size[2] * 0.995), (sx * x, y, z), M['trim'], rot=(0, 0, sx * yaw))
            clip_to(housing, shell0, (0.981, 0.980, 0.981)); members.append(housing)   # ~45 mm in: behind every lens element
            def surf_y(px, pz, back):
                sy = body_surface_y(BP, px, pz, front)
                return sy + (back if front else -back)
            def place(ob, px, pz, back):
                ob.location = (px, surf_y(px, pz, back), pz)
                clip_to(ob, shell0, (0.997, 0.997, 0.997)); members.append(ob)
            hw = size[0] * 0.42
            if front:
                for k, (ox, fn, mkey) in enumerate(((-0.13, 'LowBeam', 'lowbeam'), (0.05, 'HighBeam', 'highbeam'))):
                    rr = 0.048 if k == 0 else 0.036
                    px = sx * x + sx * ox * math.cos(yaw)
                    dish = prim_cyl(f'{gname}_{fn}Dish', rr, 0.03, (0, 0, 0), 'Y', segs=32, mat=M['chrome'], r2=rr * 0.45)
                    place(dish, px, z - 0.005, 0.028)
                    lens = prim_cyl(f'{gname}_{fn}', rr * 0.6, 0.012, (0, 0, 0), 'Y', segs=32, mat=M[mkey])
                    place(lens, px, z - 0.005, 0.010)
                # daytime running strip along the lamp top, in short segments so it follows the curve
                for k in range(7):
                    px = sx * x + sx * (-hw + 2 * hw * (k + 0.5) / 7) * math.cos(yaw)
                    seg = prim_box(f'{gname}_DRL{k}', (2 * hw / 7 + 0.004, 0.006, 0.012), (0, 0, 0), M['drl'], rot=(0, 0, sx * yaw))
                    place(seg, px, z + size[2] * 0.30, 0.012)
                # amber indicator: a bar along the lamp bottom, outer two thirds
                for k in range(2, 7):
                    px = sx * x + sx * (-hw + 2 * hw * (k + 0.5) / 7) * math.cos(yaw)
                    seg = prim_box(f'Indicator_F{tag}_{k}', (2 * hw / 7 + 0.004, 0.006, 0.014), (0, 0, 0), M['amber'], rot=(0, 0, sx * yaw))
                    place(seg, px, z - size[2] * 0.32, 0.012)
            else:
                for k in range(7):
                    px = sx * x + sx * (-hw + 2 * hw * (k + 0.5) / 7) * math.cos(yaw)
                    bar = prim_box(f'{gname}_Bar{k}', (2 * hw / 7 + 0.004, 0.01, size[2] * 0.5), (0, 0, 0), M['chrome'], rot=(0, 0, sx * yaw))
                    place(bar, px, z, 0.022)
                    seg = prim_box(f'{gname}_Tail{k}', (2 * hw / 7 + 0.004, 0.008, 0.02), (0, 0, 0), M['tail'], rot=(0, 0, sx * yaw))
                    place(seg, px, z + size[2] * 0.22, 0.010)
                # lower row: brake (outer), indicator (middle), reverse (inner)
                for k, (fn, mkey) in enumerate(((f'BrakeLamp_{tag}', 'brake'), (f'BrakeLamp_{tag}_1', 'brake'), (f'BrakeLamp_{tag}_2', 'brake'),
                                                (f'Indicator_R{tag}', 'amber'), (f'Indicator_R{tag}_1', 'amber'),
                                                (f'ReverseLamp_{tag}', 'reverse'), (f'ReverseLamp_{tag}_1', 'reverse'))):
                    px = sx * x + sx * (hw - 2 * hw * (k + 0.5) / 7) * math.cos(yaw)   # k=0 is the outer edge
                    seg = prim_box(fn, (2 * hw / 7 + 0.004, 0.008, 0.022), (0, 0, 0), M[mkey], rot=(0, 0, sx * yaw))
                    place(seg, px, z - size[2] * 0.24, 0.010)
            cover = prim_box(gname + '_Cover', (size[0] * 0.90, size[1] * 0.95, size[2] * 0.80), (sx * x, y, z), lamp_mat, rot=(0, 0, sx * yaw))
            clip_to(cover, shell0, (0.9985, 0.999, 0.9985))
            inner = shell0.copy(); inner.data = shell0.data.copy(); link(inner); inner.hide_render = False; inner.hide_viewport = False
            inner.scale = (0.9955, 0.9945, 0.9955)
            boolean_apply(cover, inner, 'DIFFERENCE'); members.append(cover)
            bpy.context.view_layer.update()
            for ob in members: _parent_keep(ob, group)
            lamp_groups[gname] = group
    recess_and_lamp('Headlamp', 0.60, -2.30, 0.575, (0.50, 0.30, 0.13), math.radians(-26), M['headlamp'])
    recess_and_lamp('Taillamp', 0.60, 2.30, 0.765, (0.52, 0.26, 0.10), math.radians(22), M['taillamp'])
    # fog lamps in the front valance corners, high-mount stop lamp under the rear glass
    for sx in (-1, 1):
        tag = 'L' if sx < 0 else 'R'
        fy = body_surface_y(BP, sx * 0.62, 0.30, True)
        prim_cyl(f'FogLamp_{tag}_Bezel', 0.05, 0.03, (sx * 0.62, fy + 0.013, 0.30), 'Y', segs=32, mat=M['trim'])    # 2 mm proud
        prim_cyl(f'FogLamp_{tag}', 0.038, 0.02, (sx * 0.62, fy + 0.006, 0.30), 'Y', segs=32, mat=M['fog'])            # lens 4 mm proud
    prim_box('HighMountStop', (0.34, 0.02, 0.022), (0, 1.66, 0.965), M['brake'])   # on the rear shelf

    # ---- grille recess + slats ----
    #
    # Problem: at 7x the near body's grille the
    # aperture carries zero mesh, texture or depth - a flat pure-black
    # rectangle. The cause is a DEPTH ORDER bug, not a material:
    #
    #   nose skin front plane   y = -2.360
    #   GrilleBack front face   y = -2.380   <- 20 mm PROUD of the nose
    #   GrilleSlat front faces  y = -2.360   <- 20 mm BEHIND the backing plate
    #   GrilleFrame_T/B         y = -2.362   <- likewise
    #
    # so the backing plate stood in front of everything it was supposed to be
    # behind. Every slat and both frame bars were sealed inside a trim-black
    # box that itself protruded from the bodywork, and the only thing a camera
    # could see of the grille was that box's flat front face. The same held
    # for the lower intake, which had no slats at all.
    #
    # The rebuild is a real recess, and it is TRIANGLE-NEGATIVE because the
    # near tier has 112 triangles of headroom against its 800,000 share (see
    # HERO_OVERLAY.triangleBudget): every bar is a plain box instead of a
    # bevelled one (a 4 mm bevel is sub-pixel at the 18 m the near tier is
    # reviewed at), which buys seven slats, a centre rib, a four-sided
    # surround and three intake blades for FEWER triangles than the three
    # buried slats and two buried bars cost.
    #
    #   nose skin               y = -2.360
    #   surround frame front    y = -2.366   (6 mm proud, the bright edge)
    #   slat / rib front faces  y = -2.352   (8 mm inside the mouth)
    #   slat / rib back faces   y = -2.322
    #   GrilleBack front face   y = -2.290   (70 mm of cavity behind the bars)
    #
    # The backing plate is oversize (0.80 x 0.21 against a 0.74 x 0.15
    # aperture) so its side faces bury in the nose instead of lying coplanar
    # with the cut walls, which would z-fight.
    gcut = prim_box('Cut', (0.74, 0.20, 0.15), (0, -2.36, 0.535))
    boolean_apply(body, gcut)
    prim_box('GrilleBack', (0.80, 0.16, 0.21), (0, -2.21, 0.535), M['trim'])
    for i in range(7):
        prim_box(f'GrilleSlat{i}', (0.72, 0.03, 0.014), (0, -2.337, 0.475 + i * 0.020), M['chrome'])
    prim_box('GrilleRib', (0.028, 0.03, 0.146), (0, -2.337, 0.535), M['chrome'])
    prim_box('GrilleFrame_T', (0.80, 0.022, 0.024), (0, -2.355, 0.622), M['chrome'])
    prim_box('GrilleFrame_B', (0.80, 0.022, 0.024), (0, -2.355, 0.448), M['chrome'])
    prim_box('GrilleFrame_L', (0.024, 0.022, 0.198), (-0.382, -2.355, 0.535), M['chrome'])
    prim_box('GrilleFrame_R', (0.024, 0.022, 0.198), (0.382, -2.355, 0.535), M['chrome'])
    # lower intake below the plate - the same recess, and it gets blades too:
    # this is the band that read as "a lower half crushed to black",
    # and a flat trim-black plate can only ever render as exactly that.
    #
    # THE CUT SPANS THE BODY/VALANCE JOINT, and that is the whole point. The
    # earlier aperture (height 0.320..0.370) was cut into the BODY only, and a
    # z-buffer from the front proves nothing of it reaches a camera: the front
    # valance's own face stands at z 2.364 across that whole band, 4 mm proud
    # of the nose skin at 2.360, so the recess was sealed behind the part in
    # front of it. Cutting the valance alone instead is no better - the body
    # skin is then the first hit at 2.360 and the pocket has no floor. Both
    # surfaces are cut, over one 0.255..0.370 band, so the mouth is a single
    # opening in whatever the camera meets first; `IntakeBack` is its floor at
    # 2.290, 70 mm behind the mouth, the same cavity depth as the grille.
    ICUT = (0.90, 0.24, 0.115)
    ICUT_AT = (0, -2.36, 0.3125)
    boolean_apply(body, prim_box('Cut', ICUT, ICUT_AT))
    prim_box('IntakeBack', (0.96, 0.16, 0.16), (0, -2.21, 0.3125), M['trim'])
    for i in range(3):
        prim_box(f'IntakeBlade{i}', (0.76, 0.03, 0.014), (0, -2.337, 0.280 + i * 0.0325), M['chrome'])
    prim_box('IntakeRib', (0.028, 0.03, 0.100), (0, -2.337, 0.3125), M['chrome'])

    # ---- fender lips: hide boolean edge and add a rolled arch edge ----
    for ay in (FRONT_Y, REAR_Y):
        for sx in (-1, 1):
            lip = arch_lip('FenderLip' + ('F' if ay < 0 else 'R') + ('L' if sx < 0 else 'R'), BP, ay, ARCH_R + 0.002, 0.012, M['paint'], proud=0.005)
            if sx < 0: lip.scale.x = -1

    # ---- wheel-well faces -> trim material ----
    body.data.materials.append(M['trim'])
    for p in body.data.polygons:
        c = p.center
        for ay in (FRONT_Y, REAR_Y):
            if abs(c.x) > W / 2 - DEPTH - 0.02 and math.hypot(c.y - ay, c.z - R) < ARCH_R + 0.003 and c.z > 0.19:
                p.material_index = 1

    # (no bevel / weighted-normal pass: both smeared the boolean cut edges into spikes; the
    #  smooth-by-angle shading from new_obj keeps the 3 mm panel gaps crisp on its own)

    # ---- valances / rockers clipped to an expanded body copy ----
    valF = prim_box('ValanceFront', (2.0, 0.40, 0.14), (0, -2.25, 0.29), M['trim']); clip_to(valF, shell0, (1.004, 1.0015, 1.004), -0.004)
    # The "lower half crushed to black" band
    # is THIS surface: a 0.14 m tall unbroken trim-black valance face across
    # the whole nose, which can only ever render as one flat mass. It is cut
    # by the SAME box the body's lower intake was cut with (`ICUT`), so the
    # two apertures are one mouth rather than two that hide each other. The
    # valance is built after the body, so the cut has to be repeated here; it
    # is the identical box, not a second aperture.
    boolean_apply(valF, prim_box('Cut', ICUT, ICUT_AT))
    valR = prim_box('ValanceRear', (2.0, 0.40, 0.14), (0, 2.25, 0.30), M['trim']); clip_to(valR, shell0, (1.004, 1.0015, 1.004), -0.004)
    for sx in (-1, 1):
        rk = prim_box('Rocker' + ('L' if sx < 0 else 'R'), (0.3, 2.30, 0.07), (sx * 0.9, 0.0, 0.215), M['gloss']); clip_to(rk, shell0, (1.004, 1.0, 1.004), -0.004)

    # ---- wheels ----
    def tyre_mesh(name):
        bm = bmesh.new()
        r_in, sh = R_RIM - 0.006, 0.05
        prof = [(-WT / 2, r_in), (-WT / 2, R - sh)]
        n = qs(6, 2)
        for i in range(1, n + 1):
            a = math.pi / 2 * i / n
            prof.append((-WT / 2 + sh - sh * math.cos(a), R - sh + sh * math.sin(a)))
        for i in range(0, n + 1):
            a = math.pi / 2 * i / n
            prof.append((WT / 2 - sh + sh * math.sin(a), R - sh + sh * math.cos(a)))
        prof.append((WT / 2, r_in))
        verts = [bm.verts.new((x, 0, r)) for x, r in prof]
        edges = [bm.edges.new((verts[i], verts[i + 1])) for i in range(len(verts) - 1)]
        edges.append(bm.edges.new((verts[-1], verts[0])))
        bmesh.ops.spin(bm, geom=verts + edges, cent=(0, 0, 0), axis=(1, 0, 0), angle=2 * math.pi, steps=qs(72, 12), use_merge=True)
        bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        return new_obj(name, bm, M['rubber'])

    def make_wheel(name, loc, flip):
        steer = bpy.data.objects.new(name, None); steer.location = loc; link(steer); steer.empty_display_size = 0.1
        parent = bpy.data.objects.new(name + '_Spin', None); link(parent); parent.parent = steer; parent.empty_display_size = 0.08
        tyre = tyre_mesh(name + '_Tyre'); tyre.parent = parent
        for g, gx in enumerate((-0.055, 0.0, 0.055)):
            groove = prim_cyl(name + f'_Groove{g}', R + 0.0005, 0.007, (gx, 0, 0), 'X', segs=72, mat=M['tread'])
            boolean_apply(groove, prim_cyl('Hole', R - 0.006, 0.02, (gx, 0, 0), 'X', segs=72))
            groove.parent = parent
        face_x = flip * (WT / 2 - 0.02)
        barrel = prim_cyl(name + '_Barrel', R_RIM - 0.004, WT - 0.12, (-flip * 0.055, 0, 0), 'X', segs=64, mat=M['darkalloy']); barrel.parent = parent
        lip = prim_cyl(name + '_Lip', R_RIM, 0.03, (face_x, 0, 0), 'X', segs=64, mat=M['alloy'])
        boolean_apply(lip, prim_cyl('Hole', R_RIM - 0.028, 0.08, (face_x, 0, 0), 'X', segs=64))
        lip.parent = parent
        hub = prim_cyl(name + '_Hub', 0.08, 0.05, (face_x, 0, 0), 'X', segs=32, mat=M['alloy']); hub.parent = parent
        hb = hub.modifiers.new('Bevel', 'BEVEL'); hb.width = 0.01; hb.segments = 3
        cap = prim_cyl(name + '_Cap', 0.038, 0.014, (face_x + flip * 0.028, 0, 0), 'X', segs=32, mat=M['chrome']); cap.parent = parent
        ln = R_RIM - 0.04
        for i in range(5):
            ang = i * 2 * math.pi / 5
            for side_off in (-0.028, 0.028):
                sp = prim_box(name + f'_Spoke{i}{"a" if side_off < 0 else "b"}', (0.045, 0.034, ln), (0, 0, 0), M['alloy'], bevel=0.007, bevel_segs=2)
                # rotate about X by ang, then offset sideways within the spoke pair
                sp.rotation_euler = (ang, 0, 0)
                sp.location = (face_x - flip * 0.012, -math.sin(ang) * ln / 2 + math.cos(ang) * side_off, math.cos(ang) * ln / 2 + math.sin(ang) * side_off)
                sp.parent = parent
        disc = prim_cyl(name + '_Disc', 0.17, 0.028, (flip * 0.03, 0, 0), 'X', segs=64, mat=M['steel']); disc.parent = parent
        cal = prim_box(name + '_Caliper', (0.06, 0.08, 0.16), (flip * 0.03, -0.11, 0.11), M['caliper'], bevel=0.01)
        cal.rotation_euler = (math.radians(-40), 0, 0); cal.parent = parent
        return parent
    for i, ay in enumerate((FRONT_Y, REAR_Y)):
        for sx in (-1, 1):
            make_wheel(f'Wheel_{"F" if i == 0 else "R"}{"L" if sx < 0 else "R"}', (sx * TRACK / 2, ay, R), sx)

    # ---- glazing: panes projected on the greenhouse ----
    t_top = GP.t_for_sx(0.80)               # windscreen lateral reach
    cutters = {}
    def pane(name, t_range, yfn, gasket_fn, span_m):
        t0, t1 = t_range; dt = (t1 - t0) * 0.07
        dtc = (t1 - t0) * 0.012 / span_m
        cut = patch('Cut' + name, GP, M['trim'], (t0 + dtc, t1 - dtc), lambda t: (yfn(t)[0] + 0.012, yfn(t)[1] - 0.012), expand=0.0, nu=28, nv=16)
        sm = cut.modifiers.new('Solid', 'SOLIDIFY'); sm.thickness = 0.10; sm.offset = 0.0; apply_all(cut)
        cutters[name] = cut
        patch(name + '_GasketLo', GP, M['gloss'], (t0, t0 + dt), gasket_fn, expand=0.003, nu=28, nv=3)
        patch(name + '_GasketHi', GP, M['gloss'], (t1 - dt, t1), gasket_fn, expand=0.003, nu=28, nv=3)
        patch(name + '_GasketA', GP, M['gloss'], t_range, lambda t: (gasket_fn(t)[0], gasket_fn(t)[0] + 0.035), expand=0.003, nu=3, nv=16)
        patch(name + '_GasketB', GP, M['gloss'], t_range, lambda t: (gasket_fn(t)[1] - 0.035, gasket_fn(t)[1]), expand=0.003, nu=3, nv=16)
        patch(name, GP, M['glass'], t_range, yfn, expand=0.006, nu=28, nv=16)
    # windscreen & rear window: y-range fixed
    pane('Windscreen', (t_top, math.pi - t_top), lambda t: (-0.78, -0.16), lambda t: (-0.83, -0.12), 1.6)
    pane('RearWindow', (t_top, math.pi - t_top), lambda t: (1.20, 1.70), lambda t: (1.16, 1.75), 1.6)
    # side windows with raked pillar edges
    for side in (1, -1):
        tag = 'R' if side > 0 else 'L'
        zlo, zhi = GP.t_for_zf(0.15, side), GP.t_for_zf(0.86, side)     # glass base 4 cm above the shoulder crease
        def zf_of(t, side=side):
            # recover vertical fraction of t on this flank
            s = math.sin(t); n = GP.n_top if s >= 0 else GP.n_bot
            return 0.5 + 0.5 * math.copysign(abs(s) ** (2 / n), s)
        def front_fn(t): zf = zf_of(t); return (-0.50 + zf * 0.55, 0.40 - zf * 0.02)
        def front_g(t): zf = zf_of(t); return (-0.55 + zf * 0.55, 0.43 - zf * 0.02)
        def rear_fn(t): zf = zf_of(t); return (0.48 + zf * 0.02, 1.00 - zf * 0.20)
        def rear_g(t): zf = zf_of(t); return (0.45 + zf * 0.02, 1.04 - zf * 0.20)
        pane('SideFront' + tag, (zlo, zhi), front_fn, front_g, 0.42)
        pane('SideRear' + tag, (zlo, zhi), rear_fn, rear_g, 0.42)

    # ---- interior ----
    interior(M, GP)

    # ---- exterior details ----
    # mirrors: a fold pivot (`Mirror_L` / `Mirror_R`) at the arm base holding arm, head, glass and repeater.
    # Rotate about Z (glTF Y): left folds with negative, right with positive.
    for sx in (-1, 1):
        tag = 'L' if sx < 0 else 'R'
        piv = bpy.data.objects.new(f'Mirror_{tag}', None); piv.location = (sx * 0.80, -0.56, 0.90); link(piv); piv.empty_display_size = 0.04
        arm = prim_box(f'Mirror_{tag}_Arm', (0.16, 0.05, 0.03), (sx * 0.87, -0.56, 0.90), M['gloss'], bevel=0.008)
        head = prim_box(f'Mirror_{tag}_Head', (0.11, 0.19, 0.11), (sx * 0.985, -0.57, 0.93), M['paint'], bevel=0.03, bevel_segs=5, rot=(0, 0, sx * math.radians(-8)))
        glass = prim_box(f'Mirror_{tag}_Glass', (0.09, 0.005, 0.085), (sx * 0.985, -0.57 + 0.095, 0.93), M['chrome'], rot=(0, 0, sx * math.radians(-8)))
        rep = prim_box(f'Indicator_M{tag}', (0.05, 0.012, 0.014), (sx * 1.0, -0.674, 0.955), M['amber'], rot=(0, 0, sx * math.radians(-8)))
        bpy.context.view_layer.update()
        for ob in (arm, head, glass, rep): _parent_keep(ob, piv)
    # door handles (hinge origin at the front end; rotate about Z to flip out) + shut lines
    for sx in (-1, 1):
        side = 'L' if sx < 0 else 'R'
        for tag, y in (('F', 0.02), ('R', 0.98)):
            hx = body_surface_x(BP, y, 0.70)
            h = prim_box(f'Handle_{tag}{side}', (0.022, 0.15, 0.03), (sx * (hx + 0.008), y, 0.70), M['paint'], bevel=0.008, bevel_segs=3)
            _set_origin_world(h, (sx * (hx + 0.002), y - 0.075, 0.70))
    def shut_line(name, y, z0, z1):
        for sx in (-1, 1):
            bm = bmesh.new()
            n = 30; vs = []
            for i in range(n):
                z = z0 + (z1 - z0) * i / (n - 1)
                x = body_surface_x(BP, y, z) + 0.001
                vs.append((bm.verts.new((sx * x, y - 0.0025, z)), bm.verts.new((sx * x, y + 0.0025, z))))
            for a, b in zip(vs, vs[1:]):
                bm.faces.new((a[0], a[1], b[1], b[0]))
            bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
            ob = new_obj(name + ('L' if sx < 0 else 'R'), bm, M['trim'])
            s = ob.modifiers.new('Solid', 'SOLIDIFY'); s.thickness = 0.002
    # bumper cover seams: across the top arc and down both flanks (door, hood and trunk edges are real gaps)
    tbF, tbR = BP.t_for_sx(0.90, -2.13), BP.t_for_sx(0.90, 2.12)
    patch('BumperSeamF', BP, M['trim'], (tbF, math.pi - tbF), lambda t: (-2.125, -2.135), expand=0.0015, nu=3, nv=40)
    patch('BumperSeamR', BP, M['trim'], (tbR, math.pi - tbR), lambda t: (2.115, 2.125), expand=0.0015, nu=3, nv=40)
    shut_line('BumperSeamSideF', -2.13, 0.30, 0.50)
    shut_line('BumperSeamSideR', 2.12, 0.30, 0.70)
    # rear diffuser fins hang below the rear valance (valance bottom is z 0.23)
    for k in range(-2, 3):
        prim_box(f'DiffuserFin{k+2}', (0.02, 0.10, 0.06), (k * 0.18, 2.27, 0.215), M['trim'])
    # character line: a slightly proud band along the flank, at a constant height (zf) along the car
    for side in (1, -1):
        for k, (ya, yb) in enumerate(((-2.08, -1.82), (-1.12, -0.625), (-0.615, 0.435), (0.445, 1.03), (1.73, 2.12))):
            patch(f'CharLine{"R" if side > 0 else "L"}{k}', BP, M['paint'], None, lambda zf, ya=ya, yb=yb: (ya, yb), expand=0.006, nu=40, nv=6, zf_range=(0.60, 0.72), side=side)
    # plates
    prim_box('PlateFront', (0.52, 0.01, 0.10), (0, -2.372, 0.405), M['plate'])
    prim_box('PlateRear', (0.52, 0.01, 0.10), (0, 2.368, 0.56), M['plate'])
    # exhaust + fin
    for sx in (-1, 1):
        prim_cyl('Exhaust' + ('L' if sx < 0 else 'R'), 0.035, 0.14, (sx * 0.55, 2.31, 0.305), 'Y', segs=32, mat=M['chrome'])
        prim_cyl('ExhaustInner' + ('L' if sx < 0 else 'R'), 0.028, 0.15, (sx * 0.55, 2.31, 0.305), 'Y', segs=32, mat=M['trim'])
    fin = prim_box('Antenna', (0.06, 0.18, 0.05), (0, 1.05, 1.44), M['paint'], bevel=0.02, bevel_segs=4)
    # wipers: pivot at the cowl end; sweep by rotating about local Z (glTF -Y)
    for sx in (-1, 1):
        w = prim_box('Wiper_' + ('L' if sx < 0 else 'R'), (0.02, 0.50, 0.012), (sx * 0.36, -0.66, 0.95), M['trim'], rot=(math.radians(33), 0, math.radians(sx * -14)))
        bpy.context.view_layer.update()
        _set_origin_world(w, w.matrix_world @ Vector((0, -0.24, 0)))
        w['rest_rot'] = tuple(w.rotation_euler)
    # underbody plate
    prim_box('Underbody', (1.40, 3.6, 0.05), (0, 0, 0.20), M['trim'])
    parts = attach_to_panels(panels)
    cut_windows(body, parts['doors'], cutters)
    roof_faces = sum(1 for p in body.data.polygons if p.center.z > 1.30 and p.normal.z > 0.5)
    assert roof_faces > 500, f'body lost its roof ({roof_faces} roof faces): a boolean failed'
    door_cards(parts['doors'], M)
    bpy.data.objects.remove(shell0, do_unlink=True)
    return body

def render(path, az_deg, el_deg, dist=7.5, target=(0, 0, 0.65), lens=50):
    sc = bpy.context.scene; cam = bpy.data.objects['Cam']; cam.data.lens = lens
    az, el = math.radians(az_deg), math.radians(el_deg)
    pos = Vector((dist * math.cos(el) * math.sin(az), -dist * math.cos(el) * math.cos(az), dist * math.sin(el) + target[2]))
    cam.location = pos
    cam.rotation_euler = (Vector(target) - pos).to_track_quat('-Z', 'Y').to_euler()
    sc.render.filepath = path
    bpy.ops.render.render(write_still=True)


# ---------------------------------------------------------------------------
# articulation: doors, hood, trunk as hinged parts; windows and wheels as pivots
# ---------------------------------------------------------------------------
def clip_to(ob, ref, scale=(1.0, 1.0, 1.0), dz=0.0):
    sh = ref.copy(); sh.data = ref.data.copy(); link(sh); sh.modifiers.clear(); sh.hide_render = False; sh.hide_viewport = False
    sh.scale = scale; sh.location.z = dz
    boolean_apply(ob, sh, 'INTERSECT')

def _copy_of(src, name):
    ob = src.copy(); ob.data = src.data.copy(); link(ob); ob.name = name
    ob.hide_render = False; ob.hide_viewport = False
    return ob

def _join_into(dst, src):
    bm = bmesh.new(); bm.from_mesh(dst.data)
    tmp = src.data.copy(); tmp.transform(dst.matrix_world.inverted() @ src.matrix_world)
    bm.from_mesh(tmp); bm.to_mesh(dst.data); bm.free()
    bpy.data.meshes.remove(tmp); bpy.data.objects.remove(src, do_unlink=True)

def _set_origin(ob, point):
    p = Vector(point)
    ob.data.transform(Matrix.Translation(-p)); ob.location = p

def _set_origin_world(ob, point):
    """Move the origin of a placed/rotated object to a world-space point without moving its surface."""
    bpy.context.view_layer.update()
    mw = ob.matrix_world.copy()
    pl = mw.inverted() @ Vector(point)
    ob.data.transform(Matrix.Translation(-pl))
    ob.matrix_world = mw @ Matrix.Translation(pl)

def _bbox_world(ob):
    return [ob.matrix_world @ Vector(c) for c in ob.bound_box]

def _parent_keep(child, parent):
    child.parent = parent; child.matrix_parent_inverse = parent.matrix_world.inverted()

PANEL_GAP = 0.003   # metres per side; the hole keeps the full box, the panel is inset by this on the masked axes

def _volume_of(me):
    bm = bmesh.new(); bm.from_mesh(me); v = abs(bm.calc_volume(signed=True)); bm.free(); return v

def _volume(ob): return _volume_of(ob.data)

def _union_boxes(boxes):
    first = boxes[0]
    for b in boxes[1:]: boolean_apply(first, b, 'UNION')
    return first

def _cut_piece(src, name, cutters, thin_refs, gap=PANEL_GAP):
    """Cut the union of `cutters` (size, loc, rot, gapmask) out of `src` (leaving a hole) and return the
    panel, inset by `gap` on the masked axes and thinned to a skin by subtracting each (ref, scale) shell."""
    # one cutter per panel: the boxes are unioned first so the solid sees a single manifold operand
    # (cutting box after box left coplanar faces that made the exact solver drop whole regions)
    # painted cutters: the gap walls carry the paint so a shut line reads as a shaded painted jamb, not a black slot
    PAINT = bpy.data.materials['VehiclePaint']
    hole = _union_boxes([prim_box('Cut', size, loc, PAINT, rot=rot) for size, loc, rot, mask in cutters])
    small = _union_boxes([prim_box('Cut', tuple(v - 2 * gap * m for v, m in zip(size, mask)), loc, PAINT, rot=rot) for size, loc, rot, mask in cutters])
    piece = _copy_of(src, name); piece.modifiers.clear()
    boolean_apply(piece, small, 'INTERSECT', keep=True)
    if len(piece.data.polygons) < 50:                       # misclassified: retry with self-intersection handling
        piece.data = src.data.copy(); boolean_apply(piece, small, 'INTERSECT', keep=True, solvers=('EXACT_SELF',))
    bpy.data.objects.remove(small, do_unlink=True)
    boolean_apply(src, hole, 'DIFFERENCE')
    for ref, scale in thin_refs:
        before = piece.data.copy()
        inner = _copy_of(ref, 'Thin'); inner.modifiers.clear(); inner.scale = scale
        boolean_apply(piece, inner, 'DIFFERENCE', keep=True)
        if _volume(piece) > 0.5 * _volume_of(before) or len(piece.data.polygons) < 50:   # kept the core instead of the skin
            old = piece.data; piece.data = before.copy(); bpy.data.meshes.remove(old)
            boolean_apply(piece, inner, 'DIFFERENCE', keep=True, solvers=('EXACT_SELF',))
        bpy.data.objects.remove(inner, do_unlink=True); bpy.data.meshes.remove(before)
    assert len(piece.data.polygons) >= 50, f'{name}: panel extraction failed'
    return piece

def extract_panels(body, shell0, M, BP):
    """Cut hood, trunk, doors and fuel door out of the pristine body; origins on their hinge lines."""
    body.name = 'Body'
    # ---- hood ----
    hood = _cut_piece(body, 'Hood', [((1.58, 1.25, 0.60), (0, -1.495, 0.80), (0, 0, 0), (1, 1, 0))], [(shell0, (0.985, 0.985, 0.985))])
    _set_origin(hood, (0, -0.87, BP.at('top', -0.87) - 0.01))
    bay = prim_box('EngineBay', (1.50, 1.24, 0.34), (0, -1.50, 0.42), M['interior']); clip_to(bay, shell0, (0.975, 0.985, 0.975))
    prim_box('EngineBlock', (0.70, 0.62, 0.26), (0, -1.55, 0.47), M['steel'], bevel=0.02)
    prim_box('EngineCover', (0.55, 0.45, 0.05), (0, -1.55, 0.625), M['trim'], bevel=0.01)
    # ---- trunk lid ----
    trunk = _cut_piece(body, 'Trunk', [((1.58, 0.31, 0.60), (0, 2.005, 0.90), (0, 0, 0), (1, 1, 0))], [(shell0, (0.985, 0.985, 0.985))])
    _set_origin(trunk, (0, 1.85, BP.at('top', 1.85) - 0.01))
    boot = prim_box('TrunkBay', (1.40, 0.62, 0.36), (0, 2.02, 0.50), M['interior']); clip_to(boot, shell0, (0.975, 0.985, 0.975))
    # ---- doors ----
    tilt = math.radians(24.6)
    doors = {}
    for sx in (-1, 1):
        side = 'L' if sx < 0 else 'R'
        # lower skin up to just below the shoulder/glasshouse crossing, a mid block over the shoulder under the
        # window frame, and the raked frame itself; the three are unioned into one panel
        for tag, y0, y1, fy0, fy1, ztop in (('F', -0.62, 0.44, -0.30, 0.44, 0.835), ('R', 0.44, 1.34, 0.44, 1.02, 0.865)):
            name = f'Door_{tag}{side}'
            lower = ((0.25, y1 - y0, ztop - 0.24), (sx * 0.925, (y0 + y1) / 2, (0.24 + ztop) / 2), (0, 0, 0), (0, 1, 1))
            mid = ((0.16, fy1 - fy0 - 0.002, 0.40), (sx * 0.88, (fy0 + fy1) / 2, 0.99), (0, 0, 0), (0, 1, 0))
            frame = ((0.16, fy1 - fy0, 0.58), (sx * 0.84, (fy0 + fy1) / 2, 1.06), (0, -sx * tilt, 0), (0, 1, 1))
            door = _cut_piece(body, name, [lower, mid, frame], [(shell0, (0.90, 1.0, 1.0))])
            hx = body_surface_x(BP, y0 + 0.02, 0.55)
            _set_origin(door, (sx * hx, y0, 0.55))
            doors[(tag, side)] = door
    # ---- fuel door on the right rear quarter (hinge at the front edge; rotate about Z, negative opens) ----
    # sits above the rear arch (arch top z 0.725) and below the shoulder roll (z ~0.85)
    fy0, fy1, fz = 1.58, 1.72, 0.79
    fuel = _cut_piece(body, 'FuelDoor', [((0.40, fy1 - fy0, 0.12), (0.90, (fy0 + fy1) / 2, fz), (0, 0, 0), (0, 1, 1))], [(shell0, (0.965, 1.0, 0.97))])
    _set_origin(fuel, (body_surface_x(BP, fy0, fz - 0.01) - 0.004, fy0, fz - 0.01))
    well = prim_box('FuelWell', (0.16, fy1 - fy0 - 0.02, 0.10), (0.86, (fy0 + fy1) / 2, fz - 0.01), M['trim']); clip_to(well, shell0, (0.985, 1.0, 0.985))
    prim_cyl('FuelCap', 0.032, 0.02, (body_surface_x(BP, (fy0 + fy1) / 2, fz - 0.01) - 0.045, (fy0 + fy1) / 2, fz - 0.01), 'X', segs=32, mat=M['gloss'])
    bpy.context.view_layer.update()  # origins moved: refresh matrix_world before parenting
    return dict(hood=hood, trunk=trunk, doors=doors, fuel=fuel)

def attach_to_panels(panels):
    """Parent windows, handles, mirrors and character-line segments to their doors."""
    doors = panels['doors']
    bpy.context.view_layer.update()
    for ob in list(root().objects):
        n = ob.name
        if n.startswith('SideFront') or n.startswith('SideRear'):
            core = n.split('_')[0]; side = core[-1]
            door = doors[('F' if 'Front' in n else 'R', side)]
            if '_Gasket' not in n:
                # roll-down pane: origin at the bottom edge so the runtime lowers it along local -Z (glTF -Y)
                ob.name = f'Window_{"F" if "Front" in n else "R"}{side}'
                bb = _bbox_world(ob)
                _set_origin_world(ob, (sum(v.x for v in bb) / 8, sum(v.y for v in bb) / 8, min(v.z for v in bb)))
                ob['rest_z'] = ob.location.z
            _parent_keep(ob, door)
        elif n.startswith('Handle_'):
            _parent_keep(ob, doors[(n[7], n[8])])
        elif n.startswith('Mirror_') and ob.type == 'EMPTY':
            _parent_keep(ob, doors[('F', n[7])])
        elif n.startswith('CharLine') and (n.endswith('2') or n.endswith('3')):
            side = n[8]
            _parent_keep(ob, doors[('F', side)] if n.endswith('2') else doors[('R', side)])
    return panels

def pose(open_state):
    """Open or close every articulated part (for review renders)."""
    for name, rot in (('Hood', (math.radians(-42), 0, 0)), ('Trunk', (math.radians(48), 0, 0)),
                      ('Door_FL', (0, 0, math.radians(58))), ('Door_RL', (0, 0, math.radians(58))),
                      ('Door_FR', (0, 0, math.radians(-58))), ('Door_RR', (0, 0, math.radians(-58)))):
        ob = bpy.data.objects.get(name)
        if ob: ob.rotation_euler = rot if open_state else (0, 0, 0)
    for n, steer in (('Wheel_FL', 25), ('Wheel_FR', 25)):
        ob = bpy.data.objects.get(n)
        if ob: ob.rotation_euler = (0, 0, math.radians(-steer)) if open_state else (0, 0, 0)
    for n in ('Wheel_FL_Spin', 'Wheel_FR_Spin', 'Wheel_RL_Spin', 'Wheel_RR_Spin'):
        ob = bpy.data.objects.get(n)
        if ob: ob.rotation_euler = (math.radians(37), 0, 0) if open_state else (0, 0, 0)
    # secondary parts: mirrors, fuel door, handles, wipers, interior
    secondary = {
        'Mirror_L': (0, 0, math.radians(-70)), 'Mirror_R': (0, 0, math.radians(70)),
        'FuelDoor': (0, 0, math.radians(-65)),
        'Handle_FL': (0, 0, math.radians(25)), 'Handle_RL': (0, 0, math.radians(25)),
        'Handle_FR': (0, 0, math.radians(-25)), 'Handle_RR': (0, 0, math.radians(-25)),
        'GloveboxLid': (math.radians(-70), 0, 0), 'ConsoleLid': (math.radians(-75), 0, 0),
        'SunVisor_L': (math.radians(-80), 0, 0), 'SunVisor_R': (math.radians(-80), 0, 0),
        'SeatBack_FL': (math.radians(-25), 0, 0), 'SeatBack_FR': (math.radians(-25), 0, 0),
        'GearLever': (math.radians(12), 0, 0), 'PedalBrake': (math.radians(-15), 0, 0), 'PedalThrottle': (math.radians(-12), 0, 0),
    }
    for name, rot in secondary.items():
        ob = bpy.data.objects.get(name)
        if ob: ob.rotation_euler = rot if open_state else (0, 0, 0)
    for name in ('SteeringWheel', 'Wiper_L', 'Wiper_R'):
        ob = bpy.data.objects.get(name)
        if not ob: continue
        base = Euler(tuple(ob['rest_rot']), 'XYZ').to_matrix().to_4x4()
        axis = 'Y' if name == 'SteeringWheel' else 'Z'      # local Y (column) for the wheel, local Z for wipers
        ob.rotation_euler = (base @ Matrix.Rotation(math.radians(40 if open_state else 0), 4, axis)).to_euler()
    for name in ('Window_FL', 'Window_FR', 'Window_RL', 'Window_RR'):
        ob = bpy.data.objects.get(name)
        if ob and 'rest_z' in ob: ob.location.z = ob['rest_z'] - (0.36 if open_state else 0.0)


# ---------------------------------------------------------------------------
# see-through glazing: openings in the greenhouse and door frames
# ---------------------------------------------------------------------------
def _cutter_box(size, loc, rot=(0, 0, 0)):
    return prim_box('Cut', size, loc, rot=rot)

def cut_windows(body, doors, cutters):
    """Every opening is the pane's own outline inset 12 mm (so the glass laps the frame), not a hand-placed box."""
    for name, cut in cutters.items():
        if name in ('Windscreen', 'RearWindow'): target = body
        else: target = doors[('F' if 'Front' in name else 'R', name[-1])]
        boolean_apply(target, cut)

def door_cards(doors, M):
    for (tag, side), door in doors.items():
        sx = -1 if side == 'L' else 1
        y0, y1 = (-0.62, 0.44) if tag == 'F' else (0.44, 1.34)
        ym = (y0 + y1) / 2; ln = (y1 - y0) - 0.08
        card = prim_box(f'DoorCard_{tag}{side}', (0.025, ln, 0.50), (sx * 0.815, ym, 0.55), M['dash'], bevel=0.008)
        insert = prim_box(f'DoorInsert_{tag}{side}', (0.012, ln * 0.7, 0.16), (sx * 0.80, ym, 0.62), M['seatInsert'], bevel=0.005)
        arm = prim_box(f'DoorArmrest_{tag}{side}', (0.08, ln * 0.45, 0.05), (sx * 0.775, ym + 0.05, 0.70), M['dash'], bevel=0.015)
        pull = prim_box(f'DoorPull_{tag}{side}', (0.03, 0.12, 0.03), (sx * 0.77, ym - 0.15, 0.74), M['chrome'], bevel=0.008)
        for ob in (card, insert, arm, pull):
            bpy.context.view_layer.update(); _parent_keep(ob, door)

# ---------------------------------------------------------------------------
# interior
# ---------------------------------------------------------------------------
# Every part that a player can touch is its own node with the origin on its
# hinge / slide axis (closed = identity):
#   Seat_FL / Seat_FR / Seat_Rear      slide pivots (translate local Y = rearward)
#   SeatBack_*                         recline (rotate about X, negative reclines)
#   SteeringWheel                      rotate about local Y (column axis; glTF local -Z)
#   GearLever, PedalBrake, PedalThrottle  rotate about X
#   GloveboxLid, ConsoleLid, SunVisor_L/R rotate about X (negative opens / drops)
#   DomeLight                          VehicleDomeLight material (switchable)
# ---------------------------------------------------------------------------
def _pivot(name, loc, rot=(0, 0, 0), size=0.05):
    ob = bpy.data.objects.new(name, None); ob.location = loc; ob.rotation_euler = rot; link(ob); ob.empty_display_size = size
    return ob

def _adopt(parent, children):
    bpy.context.view_layer.update()
    for ob in children: _parent_keep(ob, parent)

def interior(M, GP):
    # headliner: the greenhouse interior above the glass line only
    liner = loft('Headliner', GP, M['headliner'], segs=48, step=0.04, expand=-0.035, y_range=(-0.70, 1.80))   # 1 cm inside the roof skin
    boolean_apply(liner, _cutter_box((3.0, 4.0, 1.2), (0, 0.45, 0.66)))            # keep z > 1.26
    zt = GP.point(0.30, math.pi / 2, expand=-0.035)[2]
    prim_box('DomeLight', (0.16, 0.10, 0.015), (0, 0.30, zt - 0.008), M['dome'])
    for sx in (-1, 1):
        tag = 'L' if sx < 0 else 'R'
        zv = GP.point(-0.23, math.pi / 2, expand=-0.035)[2] - 0.018
        v = prim_box(f'SunVisor_{tag}', (0.30, 0.15, 0.012), (sx * 0.38, -0.155, zv), M['headliner'], bevel=0.005)
        _set_origin_world(v, (sx * 0.38, -0.23, zv + 0.006))                        # hinge at the front edge
    # floor pan, tunnel, carpet
    prim_box('FloorPan', (1.62, 2.52, 0.03), (0, 0.50, 0.35), M['carpet'])          # covers the hollowed cabin floor
    prim_box('Tunnel', (0.30, 2.10, 0.14), (0, 0.40, 0.42), M['carpet'], bevel=0.03)
    prim_box('RearShelf', (1.62, 0.46, 0.10), (0, 1.55, 0.90), M['carpet'])         # up to the rear glass base
    prim_box('RearShelfLip', (1.50, 0.03, 0.02), (0, 1.335, 0.955), M['trim'])        # front edge lip
    for k in range(-2, 3):                                                             # speaker grilles / stitch rows
        prim_box(f'RearShelfRib{k+2}', (0.22, 0.30, 0.004), (k * 0.30, 1.56, 0.952), M['seat'])
    prim_box('Firewall', (1.62, 0.04, 0.50), (0, -0.77, 0.61), M['carpet'])         # on the cabin's front wall
    # seats: slide pivot at the floor, recline pivot at the base/back junction
    def seat(name, x, y, width, bench=False):
        piv = _pivot(f'Seat_{name}', (x, y, 0.43))
        back_piv = _pivot(f'SeatBack_{name}', (x, y + 0.20, 0.57))
        base_parts = [prim_box(f'Seat_{name}_Base', (width, 0.52, 0.14), (x, y, 0.50), M['seat'], bevel=0.04, bevel_segs=4),
                      prim_box(f'Seat_{name}_BaseInsert', (width * 0.55, 0.40, 0.03), (x, y - 0.02, 0.575), M['seatInsert'], bevel=0.012)]
        back_parts = [prim_box(f'Seat_{name}_Back', (width, 0.11, 0.62), (x, y + 0.26, 0.86), M['seat'], bevel=0.035, bevel_segs=4, rot=(math.radians(-14), 0, 0)),
                      prim_box(f'Seat_{name}_BackInsert', (width * 0.55, 0.03, 0.44), (x, y + 0.215, 0.86), M['seatInsert'], bevel=0.012, rot=(math.radians(-14), 0, 0))]
        heads = [x] if not bench else [x - width * 0.30, x + width * 0.30]
        for i, hx in enumerate(heads):
            back_parts.append(prim_box(f'Seat_{name}_Headrest{i}', (0.26, 0.09, 0.15), (hx, y + 0.34, 1.20), M['seat'], bevel=0.03, bevel_segs=4, rot=(math.radians(-14), 0, 0)))
            back_parts.append(prim_box(f'Seat_{name}_HeadPost{i}', (0.12, 0.012, 0.10), (hx, y + 0.325, 1.09), M['steel']))
        if not bench:
            base_parts.append(prim_box(f'Seat_{name}_Rail', (width * 0.8, 0.50, 0.08), (x, y, 0.40), M['trim']))
        _adopt(piv, base_parts + [back_piv])
        _adopt(back_piv, back_parts)
    seat('FL', -0.36, 0.06, 0.50); seat('FR', 0.36, 0.06, 0.50)
    seat('Rear', 0.0, 0.98, 1.30, bench=True)
    # dashboard
    dash = prim_box('Dash', (1.46, 0.36, 0.18), (0, -0.57, 0.74), M['dash'], bevel=0.04, bevel_segs=4)
    prim_box('DashTop', (1.40, 0.28, 0.05), (0, -0.64, 0.845), M['dash'], bevel=0.02, bevel_segs=3)
    dash_lower = prim_box('DashLower', (1.46, 0.22, 0.24), (0, -0.50, 0.60), M['dash'], bevel=0.03)
    prim_box('Binnacle', (0.40, 0.16, 0.12), (-0.36, -0.46, 0.90), M['dash'], bevel=0.03, bevel_segs=3)
    prim_box('Dials', (0.34, 0.01, 0.08), (-0.36, -0.385, 0.90), M['dial'])
    prim_box('CentreScreen', (0.30, 0.02, 0.17), (0, -0.44, 0.92), M['screen'], bevel=0.006)
    prim_box('ScreenBezel', (0.33, 0.03, 0.20), (0, -0.45, 0.92), M['gloss'], bevel=0.008)
    # glovebox: a cavity cut into the dash face, lid hinged at its bottom edge
    boolean_apply(dash, _cutter_box((0.46, 0.20, 0.12), (0.40, -0.47, 0.72)))
    boolean_apply(dash_lower, _cutter_box((0.46, 0.20, 0.12), (0.40, -0.47, 0.72)))
    cav = prim_box('GloveboxCavity', (0.44, 0.18, 0.10), (0.40, -0.48, 0.72), M['interior'])
    boolean_apply(cav, _cutter_box((0.42, 0.20, 0.08), (0.40, -0.46, 0.72)))
    lid = prim_box('GloveboxLid', (0.48, 0.02, 0.14), (0.40, -0.395, 0.72), M['dash'], bevel=0.008)
    _set_origin_world(lid, (0.40, -0.395, 0.65))
    for i in range(4):
        prim_cyl(f'Vent{i}', 0.035, 0.02, (-0.62 + i * 0.41, -0.395, 0.84), 'Y', segs=24, mat=M['trim'])
    # centre console with a lidded bin, gear lever on a pivot
    console = prim_box('Console', (0.30, 0.80, 0.16), (0, -0.05, 0.55), M['dash'], bevel=0.02, bevel_segs=3)
    boolean_apply(console, _cutter_box((0.24, 0.26, 0.10), (0, 0.18, 0.60)))
    bin_ = prim_box('ConsoleBin', (0.235, 0.255, 0.09), (0, 0.18, 0.595), M['interior'])
    boolean_apply(bin_, _cutter_box((0.215, 0.235, 0.09), (0, 0.18, 0.61)))
    clid = prim_box('ConsoleLid', (0.26, 0.28, 0.05), (0, 0.18, 0.645), M['seat'], bevel=0.015)
    _set_origin_world(clid, (0, 0.32, 0.62))                                          # hinge at the rear edge
    lever = _pivot('GearLever', (0, -0.22, 0.62))
    _adopt(lever, [prim_cyl('GearLever_Stalk', 0.012, 0.16, (0, -0.22, 0.70), 'Z', segs=12, mat=M['chrome']),
                   prim_box('GearLever_Knob', (0.05, 0.06, 0.05), (0, -0.22, 0.79), M['seat'], bevel=0.02, bevel_segs=4)])
    # steering wheel on the column axis (left-hand drive)
    col = prim_cyl('SteeringColumn', 0.03, 0.30, (0, 0, 0), 'Y', segs=16, mat=M['dash'])
    col.location = (-0.36, -0.36, 0.86); col.rotation_euler = (math.radians(-25), 0, 0)
    wheel = _pivot('SteeringWheel', (-0.36, -0.25, 0.92), rot=(math.radians(-25), 0, 0))
    wheel['rest_rot'] = tuple(wheel.rotation_euler)
    rim = torus_x('SteeringWheel_Rim', 0.185, 0.018, (0, 0, 0), M['seat'], steps=48, ring=12)
    rim.rotation_euler = (math.radians(65), 0, math.radians(90)); rim.location = (-0.36, -0.25, 0.92)
    hub = prim_cyl('SteeringWheel_Hub', 0.06, 0.05, (0, 0, 0), 'Y', segs=24, mat=M['dash'])
    hub.location = (-0.36, -0.25, 0.92); hub.rotation_euler = (math.radians(-25), 0, 0)
    wheel_parts = [rim, hub]
    for i, ang in enumerate((0, 120, 240)):
        sp = prim_box(f'SteeringWheel_Spoke{i}', (0.04, 0.02, 0.17), (0, 0, 0), M['dash'], bevel=0.006)
        a = math.radians(ang + 90)
        local = Vector((math.cos(a) * 0.085, 0, math.sin(a) * 0.085))
        world = Matrix.Rotation(math.radians(-25), 4, 'X') @ local
        sp.location = Vector((-0.36, -0.25, 0.92)) + Vector((world.x, world.y, world.z))
        sp.rotation_euler = (Matrix.Rotation(math.radians(-25), 4, 'X') @ Matrix.Rotation(-a + math.pi / 2, 4, 'Y')).to_euler()
        wheel_parts.append(sp)
    _adopt(wheel, wheel_parts)
    # pedals hang from their top edge; rear-view mirror
    for name, px in (('PedalBrake', -0.46), ('PedalThrottle', -0.30)):
        p = prim_box(name, (0.06, 0.02, 0.09), (px, -0.72, 0.46), M['trim'], rot=(math.radians(20), 0, 0))
        bpy.context.view_layer.update()
        _set_origin_world(p, p.matrix_world @ Vector((0, 0, 0.045)))
    prim_box('RearViewMirror', (0.22, 0.03, 0.06), (0, -0.40, 1.28), M['gloss'], bevel=0.01, rot=(math.radians(20), 0, 0))
