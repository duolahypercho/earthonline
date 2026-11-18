# Hand ornament for the 1923 blue-grey flats (sf-building-259974357): the grey
# marble frame round the central entry, two jamb posts and a lintel.
L = kit.L
MARBLE = 'cast-stone-trim@a8adb4'
fc = 0.44                                   # door centre, style layout 0.36..0.52
half = (0.52 - 0.36) * L / 2
for sd in (-1, 1):
    kit.box(fc, 1.75, 0.06, 0.24, 3.5, 0.12, MARBLE, dr=sd * (half + 0.1))
kit.box(fc, 3.55, 0.06, 2 * half + 0.44, 0.24, 0.14, MARBLE)
