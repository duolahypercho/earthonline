# Hand ornament, Hyde 1959 apartment block (photo-checked).
# 1) Dark-brown rectangular lattice screen over the entry core, above the gate to the roof.
L = kit.L
F0, F1 = 0.27, 0.45
fc = (F0 + F1) / 2
w = (F1 - F0) * L
top = 9.4
y0 = 2.95
n = int(w / 0.3)
for k in range(n + 1):                                   # verticals
    kit.box(fc, (y0 + top) / 2, 0.05, 0.06, top - y0, 0.07, 'wood-siding@3a2a20', dr=(k / n - 0.5) * (w - 0.1))
y = y0
while y <= top:                                          # horizontals: a grid of rectangular openings
    kit.box(fc, y, 0.04, w, 0.06, 0.06, 'wood-siding@3a2a20')
    y += 0.5
# 2) Visible stair behind the glazed tower: a zig-zag of steps per storey.
fs = 0.185
for base in (0.0, 3.4, 6.57):
    for i in range(8):
        d = 1 if base != 3.4 else -1
        kit.box(fs, base + 0.3 + i * 0.36, -0.3, 0.3, 0.07, 0.7, 'painted-steel@3a3a3e', dr=d * (-0.9 + i * 0.25))
# 3) Free-standing red-topped pipe-frame canopy at the kerb side (not attached to the facade).
fcn = (F0 + F1) / 2
kit.box(fcn, 3.05, 2.0, w + 0.3, 0.06, 1.5, 'painted-steel@a02a28')
for dr in (-w / 2, w / 2):
    for off in (1.3, 2.7):
        kit.column(fcn, 0.0, 3.0, off, 0.035, 'painted-steel@8a8a8a', dr=dr)
