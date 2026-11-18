# Tall ornamented terracotta piers between the window bays and an arcade cap (street photo).
C = 'cast-stone-trim@dccdaa'
D = 'cast-stone-trim@bfae88'
for f in (0.51, 0.635, 0.76, 0.885, 0.995):
    kit.box(f, 12.5, 0.0, 0.9, 16.0, 0.3, C)
    kit.box(f, 12.5, 0.16, 0.3, 16.0, 0.04, D)        # carved centre strip
    kit.box(f, 4.55, 0.0, 1.1, 0.3, 0.42, D)           # capital at the arcade
