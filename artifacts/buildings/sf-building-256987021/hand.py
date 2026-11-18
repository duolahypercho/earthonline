# Maroon-pink painted-brick plinth under the buff-brick ground storey (photo), except at the door.
PLINTH = 'brick-running-bond@e0a5ac'
for (a, b) in ((0.0, 0.57), (0.69, 1.0)):
    kit.box((a + b) / 2, 0.6, 0.0, (b - a) * kit.L, 1.2, 0.04, PLINTH)
