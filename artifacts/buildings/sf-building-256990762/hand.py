# Small balconies with maroon fretwork rails between the two bay stacks (photo: cream flats,
# maroon panelled railing across the gap at the foot of each bay storey).
RAIL = 'painted-white@8c3a3a'
CREAM = 'painted-white@e6dcc0'
for y in (3.0 + 2.7, 3.0 + 2 * 2.7):          # foot of storeys 2 and 3
    kit.box(0.5, y - 0.06, 0.0, 1.9, 0.1, 0.55, CREAM)
    kit.box(0.5, y + 0.4, 0.5, 1.9, 0.8, 0.05, RAIL)
# Dark wood entry door slab in the recessed stair entry (photo: dark door), centred on the door span.
kit.box(0.70, 1.1, 0.02, 1.5, 2.2, 0.05, 'wood-siding@3a2218')
