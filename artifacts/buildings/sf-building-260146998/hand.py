# Relief ornament seen in the street photo: concentric-disc surround of the arched entry,
# keystone crest, frieze beads and spandrel panels between the paired windows.
R = 'cast-stone-trim@cdbc94'
D = 'cast-stone-trim@a89870'
for f in (0.435, 0.455, 0.625, 0.645):
    for k in range(10):
        kit.box(f, 0.5 + k * 0.4, 0.07, 0.34, 0.34, 0.04, R if k % 2 == 0 else D)
kit.box(0.54, 4.2, 0.1, 0.5, 0.5, 0.06, D)       # crest at the keystone
for i in range(40):
    kit.box(0.02 + 0.96 * i / 39, 4.75, 0.06, 0.3, 0.18, 0.04, R)   # frieze relief
for k in range(1, 5):
    y = 4.689 + 3.39 * (k - 1) + 3.4
    for f in (0.21, 0.41, 0.565, 0.73, 0.925):
        kit.box(f, y, 0.03, 0.5, 0.8, 0.05, R)
