# Hand ornament for the 1926 cream flats, Hyde near Ellis (sf-building-71209427).
# The photo shows small teal ornamental panels on the spandrels above the
# window row of each canted bay. One plain teal block per bay per storey; no
# relief is modelled and no artwork copied.
L = kit.L
LV = [0.0, 4.062, 7.03, 9.998, 12.966, 15.934, 18.902]   # ground 4.062, uppers 2.968
TEAL = 'painted-white@3f9a92'
for f in (0.36, 0.5, 0.77, 0.91):          # the four bay centres in style.json
    for k in (3, 4, 5, 6):                  # spandrel under each bay storey above the first
        kit.box(f, LV[k] - 0.5, 0.72, 0.7, 0.5, 0.05, TEAL)
