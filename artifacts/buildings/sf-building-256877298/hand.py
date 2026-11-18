# Hand ornament, Hyde 1907 pink shop building (photo-checked).
import math
# Clay-tile hoods over the two end bays
for f in (0.15, 0.85):
    kit.gable(f, 8.55, 3.0, 0.95, 0.45, 'asphalt-roof@b04a2c', 'painted-white@eba58f')
# Oval teal cartouche (frame ring) with a leafy green emblem between the bays
cy = 7.0
for k in range(24):
    a = 2 * math.pi * k / 24
    kit.box(0.5, cy + 0.9 * math.sin(a), 0.03, 0.13, 0.13, 0.03, 'painted-white@4f9a90', dr=0.62 * math.cos(a))
kit.box(0.5, cy, 0.03, 0.4, 0.8, 0.03, 'painted-white@4a8a50')
