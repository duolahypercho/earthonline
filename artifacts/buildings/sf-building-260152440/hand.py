# Red-bordered spandrel panels in the wall between the bay stacks (sf-building-260152440).
# The street photo shows a pink panel with a red border above each window row.
RED = 'painted-white@9c3a30'
PINK = 'painted-stucco@dba596'
levels = (4.684, 8.071, 11.458, 14.845)     # storey plates 1..4 (spandrel sits above each head)
for lv in levels:
    y = lv + 3.29
    for f in (0.40, 0.675):
        kit.box(f, y, 0.0, 1.15, 0.9, 0.05, RED)
        kit.box(f, y, 0.05, 0.9, 0.65, 0.03, PINK)
