# Deco front (sf-building-260152445). The steel-blue front (0.485-0.645 of the
# footprint, wall colour from style wallZones) is flanked by two full-height
# fluted piers capped with round medallion stand-ins, has a moulded frieze
# under a stepped crest, a spandrel panel and a louvre vent beside the gate.
BLUE = 'painted-stucco@7a90b8'
BLUE2 = 'painted-stucco@6a82ac'
DARK = 'painted-steel@2c3038'
H = 9.3
for f in (0.485, 0.645):
    kit.box(f, H / 2, 0.0, 0.7, H, 0.14, BLUE)                 # full-height pier
    for dr in (-0.2, 0.0, 0.2):
        kit.box(f, H / 2 - 0.3, 0.0, 0.07, H - 0.8, 0.19, BLUE2, dr=dr)   # vertical fluting
    kit.box(f, H + 0.2, 0.0, 0.5, 0.4, 0.12, BLUE)             # pier cap
    kit.box(f, H + 0.5, 0.0, 0.34, 0.34, 0.12, BLUE2)          # medallion stand-in
kit.box(0.565, 4.7, 0.0, 6.0, 0.7, 0.08, BLUE2)               # spandrel panel
kit.box(0.565, H - 0.45, 0.0, 6.0, 0.5, 0.1, BLUE2)           # moulded frieze
kit.box(0.565, H - 0.8, 0.0, 6.0, 0.12, 0.14, BLUE)           # frieze lower moulding
kit.box(0.565, H + 0.15, 0.0, 3.0, 0.3, 0.1, BLUE)            # stepped crest
kit.box(0.565, H + 0.4, 0.0, 1.6, 0.2, 0.1, BLUE)
kit.box(0.617, 1.4, 0.0, 0.7, 1.5, 0.06, DARK)                # louvred vent panel
