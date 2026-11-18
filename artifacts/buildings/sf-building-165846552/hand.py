# Vertical ribbed cladding strips between the window pairs (street photo: fluted champagne panels).
for i in range(13):
    f = i / 12
    f = min(max(f, 0.004), 0.996)
    kit.box(f, 12.5, 0.0, 0.7, 17.0, 0.07, 'painted-stucco@bda775')
    for d in (-0.22, 0.22):
        kit.box(f, 12.5, 0.04, 0.12, 17.0, 0.05, 'painted-stucco@a99362', dr=d)
