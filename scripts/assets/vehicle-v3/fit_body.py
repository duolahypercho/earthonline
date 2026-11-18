"""Fit a v3 body JSON from a side trace.

python3 fit_body.py --trace <x_metric.json> --template <bodies/tpl.json> --id sedan
        --L 4.85 --W 1.84 --H 1.45 --R 0.34 --FA -1.40 --RA 1.42 [--track 1.58] [--out bodies/sedan.json]

The trace (y, z) lists come from a side photograph scaled to the catalogue
footprint. Curves become PCHIP control points via Douglas-Peucker; the wheel
arches are circle fits; glass, doors and lamps are placed from the traced glass
extents. Everything not measurable from one side view (plan widths, lamp
outlines, crease depth) comes from the template, scaled to the footprint. The
result is a draft to review against the reference, not an approved body.
"""
import argparse
import json
import math
from pathlib import Path

HERE = Path(__file__).resolve().parent


def rdp(pts, eps):
    if len(pts) < 3:
        return pts
    (x0, y0), (x1, y1) = pts[0], pts[-1]
    dx, dy = x1 - x0, y1 - y0
    L = math.hypot(dx, dy) or 1e-9
    dmax, idx = 0.0, 0
    for i in range(1, len(pts) - 1):
        d = abs(dy * pts[i][0] - dx * pts[i][1] + x1 * y0 - y1 * x0) / L
        if d > dmax:
            dmax, idx = d, i
    if dmax > eps:
        return rdp(pts[:idx + 1], eps)[:-1] + rdp(pts[idx:], eps)
    return [pts[0], pts[-1]]


def smooth(pts, k=5):
    out = []
    for i in range(len(pts)):
        win = sorted(p[1] for p in pts[max(0, i - k):i + k + 1])
        out.append([pts[i][0], win[len(win) // 2]])
    return out


def gsmooth(pts, sigma=6):
    """Gaussian smoothing over sample index: removes the pixel staircase of a traced
    edge, which a monotone spline would otherwise hold as flat steps (ripples)."""
    import math as _m
    n = len(pts)
    r = int(3 * sigma)
    w = [_m.exp(-0.5 * (k / sigma) ** 2) for k in range(-r, r + 1)]
    out = []
    for i in range(n):
        acc = ws = 0.0
        for k, wk in zip(range(-r, r + 1), w):
            j = i + k
            if 0 <= j < n:
                acc += pts[j][1] * wk
                ws += wk
        out.append([pts[i][0], acc / ws])
    return out


def dedupe(pts):
    out = []
    for y, z in sorted(pts):
        if out and abs(y - out[-1][0]) < 1e-4:
            continue
        out.append([round(y, 4), round(z, 4)])
    return out


def interp(pts, y):
    if y <= pts[0][0]:
        return pts[0][1]
    for (a, za), (b, zb) in zip(pts, pts[1:]):
        if a <= y <= b:
            return za + (zb - za) * (y - a) / max(1e-9, b - a)
    return pts[-1][1]


def circle_fit(pts):
    # algebraic least squares (Kasa) without numpy
    n = len(pts)
    sx = sum(p[0] for p in pts); sy = sum(p[1] for p in pts)
    sxx = sum(p[0] ** 2 for p in pts); syy = sum(p[1] ** 2 for p in pts); sxy = sum(p[0] * p[1] for p in pts)
    sxxx = sum(p[0] ** 3 for p in pts); syyy = sum(p[1] ** 3 for p in pts)
    sxyy = sum(p[0] * p[1] ** 2 for p in pts); sxxy = sum(p[0] ** 2 * p[1] for p in pts)
    A = [[sxx, sxy, sx], [sxy, syy, sy], [sx, sy, n]]
    b = [0.5 * (sxxx + sxyy), 0.5 * (sxxy + syyy), 0.5 * (sxx + syy)]
    # solve 3x3
    import copy
    M = copy.deepcopy(A)
    for i in range(3):
        M[i].append(b[i])
    for i in range(3):
        p = max(range(i, 3), key=lambda r: abs(M[r][i]))
        M[i], M[p] = M[p], M[i]
        for r in range(3):
            if r != i:
                f = M[r][i] / M[i][i]
                for c in range(i, 4):
                    M[r][c] -= f * M[i][c]
    cx, cy, c = (M[i][3] / M[i][i] for i in range(3))
    r = math.sqrt(max(1e-9, c + cx * cx + cy * cy - 2 * c + c)) if False else math.sqrt(max(1e-9, cx * cx + cy * cy + (2 * c - cx * cx - cy * cy) - c))
    # recompute radius directly as the mean distance
    r = sum(math.hypot(p[0] - cx, p[1] - cy) for p in pts) / n
    return cx, cy, r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--trace', required=True)
    ap.add_argument('--template', required=True)
    ap.add_argument('--id', required=True)
    ap.add_argument('--label', default=None)
    for k in ('L', 'W', 'H', 'R', 'FA', 'RA'):
        ap.add_argument('--' + k, type=float, required=True)
    ap.add_argument('--track', type=float, default=None)
    ap.add_argument('--out', default=None)
    a = ap.parse_args()
    t = json.loads(Path(a.trace).read_text())
    tpl = json.loads(Path(a.template).read_text())
    L, W, H, R, FA, RA = a.L, a.W, a.H, a.R, a.FA, a.RA
    y0, y1 = -L / 2, L / 2
    top = smooth(dedupe(t['top']), 3)
    top = [p for p in top if y0 - 0.01 <= p[0] <= y1 + 0.01]
    topc = rdp(gsmooth(top, 5), 0.005)
    # sill / bumper underside: paint bottom away from the arches
    pb = dedupe(t['pbot'])
    arches = []
    for ax in (FA, RA):
        pts = [p for p in pb if abs(p[0] - ax) < R + 0.12 and p[1] > R * 1.1]
        if len(pts) > 8:
            cy_fixed = R
            # fit radius with the centre pinned to the axle (the arch is concentric with the wheel)
            r = sum(math.hypot(p[0] - ax, p[1] - cy_fixed) for p in pts if p[1] > R + 0.1) / max(1, len([p for p in pts if p[1] > R + 0.1]))
            arches.append([ax, R, round(min(max(r - 0.02, R + 0.035), R + 0.065), 4)])
        else:
            arches.append([ax, R, round(R + 0.05, 4)])
    if len(pb) < 0.6 * len(t['bot']) or tpl.get('fit', {}).get('sillFromSilhouette'):
        # paint mask unreliable (dark or reflective paint): body underside from the silhouette
        pb = [p for p in dedupe(t['bot']) if all(abs(p[0] - ax) > R + 0.10 for ax in (FA, RA))]
        arches = [[ax, R, round(R + tpl.get('fit', {}).get('archGap', 0.05), 4)] for ax in (FA, RA)]
    zs_ = sorted(p[1] for p in pb)
    base_ = zs_[len(zs_) // 5]
    # near an axle, a paint bottom well above the sill is arch or cladding, not body underside
    sill = [p for p in pb if all(abs(p[0] - ar[0]) > ar[2] + 0.02 for ar in arches)
            and not (min(abs(p[0] - ar[0]) for ar in arches) < 0.75 and p[1] > base_ + 0.10)]
    # beyond the arches the underside is the bumper; lower bumpers are often unpainted
    # (black) plastic, so take those spans from the silhouette, not the paint
    sil = dedupe(t['bot'])
    ends = [p for p in sil if (p[0] < FA - R - 0.20 or p[0] > RA + R + 0.20)]
    sill = [p for p in sill if FA - R - 0.20 <= p[0] <= RA + R + 0.20] + ends
    sill = smooth(sorted(sill), 5)
    bottom = rdp(sill, 0.006)
    # the silhouette rounds off in the last few centimetres; a bumper underside does
    # not climb 7 cm there, so hold the ends to a gentle rise
    inner_b = [p for p in bottom if y0 + 0.12 < p[0] < y1 - 0.12]
    zf, zr_ = inner_b[0][1], inner_b[-1][1]
    bottom = [[y0, round(zf + 0.05, 4)]] + inner_b + [[y1, round(zr_ + 0.06, 4)]]
    # glass line and roof rail from the traced glass extents
    dtop, dbot = dedupe(t['dtop']), dedupe(t['dbot'])
    g0, g1 = dbot[0][0], dbot[-1][0]
    beltc = rdp(smooth(dbot, 6), 0.006)
    # outside the glass the belt follows the fender line just under the bonnet / deck
    # outside the glass the belt is the bonnet / deck edge: a fixed drop under the top line
    fwd = [[y, round(interp(topc, y) - 0.06, 4)] for y in [y0 + k * 0.1 for k in range(int((g0 - 0.2 - y0) / 0.1) + 1)]]
    aft = [[y, round(min(interp(beltc, g1) + 0.02, interp(topc, y) - 0.05), 4)] for y in [g1 + 0.15 + k * 0.1 for k in range(int((y1 - g1 - 0.15) / 0.1) + 1)]]
    belt = fwd + beltc + aft
    belt = dedupe([p for p in belt if y0 <= p[0] <= y1])
    raild = []
    for y, z in rdp(smooth(dtop, 6), 0.006):
        raild.append([y, round(max(0.018, interp(topc, y) - z + 0.012), 4)])
    rail = dedupe([[y0, 0.05]] + raild + [[y1, 0.05]])
    rail = [[y, min(z, 0.09)] for y, z in rail]
    crease_drop = tpl.get('fit', {}).get('creaseDrop', 0.16)
    crease = [[y, round(z - crease_drop, 4)] for y, z in belt]

    cfg = json.loads(json.dumps(tpl))
    cfg.pop('fit', None)
    cfg.update(id=a.id, label=a.label or tpl['label'], L=L, W=W, H=H, R=R, frontAxle=FA, rearAxle=RA,
               track=a.track or round(W - 0.23, 3), arches=arches)
    cfg['note'] = ('Curves in metres, Blender frame (-Y nose). Fitted by fit_body.py from a side trace of a '
                   'production car scaled to the catalogue footprint; plan widths and lamp outlines from the '
                   'template. Reference identity is kept off-repository.')
    sx = W / tpl['W']
    sz = H / tpl['H']
    c = cfg['curves']
    c['top'], c['bottom'], c['belt'], c['rail'], c['crease'] = topc, bottom, belt, rail, crease
    # plan widths from the template, rescaled in y to the new length and in x to the new width
    ly = L / tpl['L']
    for k in ('halfW', 'beltW', 'railW', 'crown', 'bulgeDrop', 'tuck'):
        if k in c:
            c[k] = [[round(y * ly, 4), round(v * (sx if k.endswith('W') else 1), 4)] for y, v in tpl['curves'][k]]
    if tpl.get('boot') == 'trunk':
        # a saloon deck edge runs nearly full width; the roof rail narrows only over the glass
        hw_ = W / 2
        roof_w = interp(c['railW'], 0.0)
        c['railW'] = [p for p in c['railW'] if p[0] < 0.3] + [[0.6, roof_w], [1.0, roof_w + 0.02]]
        c['railW'] += [[y1 - 1.0, hw_ * 0.84], [y1 - 0.3, hw_ * 0.86], [y1, hw_ * 0.80]]
        c['railW'] = sorted(c['railW'])
        c['beltW'] = sorted([p for p in c['beltW'] if p[0] < 0.8] + [[y1 - 1.0, hw_ * 0.93], [y1 - 0.3, hw_ * 0.93], [y1, hw_ * 0.86]])
    # openings, doors and lids placed from the glass extents and the axles
    ar_f, ar_r = arches[0][2], arches[1][2]
    d0 = round(FA + ar_f + 0.06, 3)
    r1 = round(RA - ar_r - 0.02, 3)
    frac = tpl.get('fit', {}).get('bPillar', 0.55)
    d1 = round(d0 + frac * (r1 - d0) + 0.02, 3)
    cfg['doors']['front'] = [d0, d1]
    cfg['doors']['rear'] = [round(d1 + 0.005, 3), r1]
    belt_at = lambda y: interp(belt, y)
    cfg['doors']['rearEdge'] = [[r1, 0.25], [round(r1 + 0.18, 3), round(belt_at(r1 + 0.18), 3)], [round(r1 + 0.22, 3), round(H + 0.05, 3)]]
    cfg['doors']['bottomZ'] = round(interp(bottom, 0.0) + 0.10, 3)
    cfg['glass'] = dict(front=round(max(g0, d0) + 0.03, 3), bFront=round(d1 - 0.035, 3), bRear=round(d1 + 0.04, 3),
                        rear=round(r1 + 0.13, 3), qFront=round(r1 + 0.19, 3), qRear=round(min(g1, y1 - 0.5), 3))
    q_cap = tpl.get('fit', {}).get('quarterEnd')  # set after the roof line is known
    # lids: the windscreen base is where the roof line starts climbing out of the bonnet
    cowl = tpl['cowl'] * ly
    for y, z in top:
        if y > y0 + 0.4 and y < 0 and (interp(top, y + 0.10) - z) > 0.045:
            cowl = round(y, 3)
            break
    cfg['cowl'] = cowl
    peak = max(top, key=lambda p: p[1])
    roofz = peak[1]
    rf = next((p[0] for p in top if p[0] > cowl and p[1] > roofz - 0.03), peak[0])
    rr = next((p[0] for p in reversed(top) if p[1] > roofz - 0.05), peak[0])
    cfg['roofFront'], cfg['roofRear'] = round(rf, 3), round(rr, 3)
    cfg['hatchBase'] = round(min(y1 - 0.12, rr + tpl['hatchBase'] - tpl['roofRear']), 3)
    if tpl.get('boot') == 'trunk':
        # backlight bottom: first point behind the roof where the profile flattens into the deck
        for i in range(len(top) - 1):
            y, z = top[i]
            if y > rr + 0.25:
                y2, z2 = top[min(len(top) - 1, i + 8)]
                if y2 > y and abs((z2 - z) / (y2 - y)) < 0.28:
                    cfg['hatchBase'] = round(y, 3)
                    break
    cfg['glass']['qRear'] = round(min(cfg['glass']['qRear'], rr - 0.02 + (q_cap or 0.0)), 3)
    for k in ('hoodFront', 'tailCut', 'mirrorY'):
        if k in tpl:
            cfg[k] = round(tpl[k] * ly, 3)
    cfg['mirrorY'] = round(d0 + 0.10, 3)
    # lamp, grille and tailgate outlines: remapped inside each fascia's own span
    # (bumper underside to bonnet/deck top at that end), not scaled from the ground
    def span(curves, L_, end):
        y = (-L_ / 2 + 0.10) if end == 'front' else (L_ / 2 - 0.10)
        yt = (-L_ / 2 + 0.10) if end == 'front' else (L_ / 2 - 0.30)
        return interp(curves['bottom'], y), interp(curves['top'], yt)
    t_front, t_rear = span(tpl['curves'], tpl['L'], 'front'), span(tpl['curves'], tpl['L'], 'rear')
    n_front, n_rear = span(c, L, 'front'), span(c, L, 'rear')
    def remap(z, src, dst):
        return dst[0] + (z - src[0]) / max(1e-6, src[1] - src[0]) * (dst[1] - dst[0])
    for k, pts in tpl['lamps'].items():
        src, dst = (t_rear, n_rear) if k == 'tail' else (t_front, n_front)
        cfg['lamps'][k] = [[round(x * sx, 3), round(remap(z, src, dst), 3)] for x, z in pts]
    for k in ('hatchOutline', 'rearWindowOutline'):
        if k in tpl:
            cfg[k] = [[round(x * sx, 3), round(remap(z, t_rear, n_rear), 3)] for x, z in tpl[k]]
    cfg['plates'] = {'front': [round(y0 - 0.012, 3), round(remap(tpl['plates']['front'][1], t_front, n_front), 3)],
                     'rear': [round(y1 - 0.02, 3), round(remap(tpl['plates']['rear'][1], t_rear, n_rear), 3)]}
    cfg['frontLipZ'] = round(n_front[0] + 0.07, 3)
    cfg['rearDiffuserZ'] = round(n_rear[0] + 0.08, 3)
    # squared ends: a flat end panel with a tight corner roll (see sedan, 2026-09-23)
    cfg.update(tailRound=0.14, tailPower=3.0, tailMin=0.85, tailDome=0.0,
               noseRound=0.30, nosePower=4.0, noseMin=0.55, noseDome=0.01)
    out = Path(a.out) if a.out else HERE / 'bodies' / f'{a.id}.json'
    out.write_text(json.dumps(cfg, indent=1))
    print('wrote', out, 'arches', arches, 'cowl', cowl, 'roof', rf, rr, 'glass', g0, g1)


main()
