# Hand ornament: buff stone quoin surrounds on storey 1, lintel caps on storey 2,
# and the small iron balcony across the double window at the upper right.
S='painted-white@d9c7a0'; IRON='painted-steel@222222'
G=4.963; U=3.436; L=kit.L
for (a, b) in ((0.40,0.47),(0.505,0.575),(0.62,0.73),(0.79,0.87)):
    fc=(a+b)/2; w=(b-a)*L
    for n in range(6):                      # alternating quoin blocks both jambs
        wq = 0.34 if n%2==0 else 0.2
        for sgn in (-1,1):
            kit.box(fc, G+0.9+n*0.28, 0.0, wq, 0.26, 0.06, S, dr=sgn*(w/2+wq/2-0.02))
    kit.box(fc, G+2.55, 0.0, w+0.8, 0.16, 0.08, S)       # lintel
    kit.box(fc, G+0.82, 0.0, w+0.5, 0.1, 0.12, S)        # sill
for f in (0.165,0.33,0.44,0.54,0.64,0.82):
    kit.box(f, G+U+2.45, 0.0, 1.15, 0.12, 0.1, S)       # storey-2 head caps
    kit.box(f, G+U+0.8, 0.0, 1.1, 0.08, 0.1, S)
# iron balcony, storey 2, right of centre
y0=G+U+0.15
kit.box(0.83, y0, 0.0, 2.3, 0.14, 0.7, IRON)
kit.box(0.83, y0+0.95, 0.62, 2.3, 0.06, 0.06, IRON)
for n in range(15):
    kit.box(0.83, y0+0.5, 0.64, 0.025, 0.9, 0.025, IRON, dr=-1.1+n*(2.2/14))
