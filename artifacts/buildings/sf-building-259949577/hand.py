# Hand ornament: medallion panels under each bay, an oval window beside the arched entry,
# and a belt course over the ground.
C='painted-white@efe7d2'
G=3.4; U=3.4; L=kit.L
kit.box(0.5, 3.4, 0.0, kit.L, 0.22, 0.18, 'painted-white@efe7d2')   # heavy belt course
kit.box(0.5, 3.24, 0.0, kit.L-0.2, 0.1, 0.07999999999999999, 'painted-white@efe7d2')   # soffit step
for f in (0.26,0.69):
    kit.box(f, G+U-0.35, 0.0, 2.0, 0.5, 0.1, C)      # apron panel under the bay
    kit.box(f, G+U-0.35, 0.1, 0.34, 0.34, 0.03, C)  # medallion
# oval window, ground-floor lintel level, beside the arched entry
kit.box(0.565, G+1.3, 0.0, 0.8, 1.1, 0.05, 'glass-clear')
for dr, w, hh in ((-0.4,0.1,1.1),(0.4,0.1,1.1)):
    kit.box(0.565, G+1.3, 0.05, w, hh, 0.06, C, dr=dr)
kit.box(0.565, G+1.85, 0.05, 0.8, 0.1, 0.06, C)
kit.box(0.565, G+0.75, 0.05, 0.8, 0.1, 0.06, C)
