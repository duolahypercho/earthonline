# Belt course with brackets and ornamented bay aprons (sf-building-260142629).
# The street photo shows a reddish-brown belt under the bay storeys, small
# brackets under it, and a carved panel on the foot of each bay.
BELT = 'painted-stucco@8a4a3c'
PANEL = 'painted-stucco@b8644f'
kit.box(0.5, 6.78, 0.0, 8.9, 0.34, 0.16, BELT)
for i in range(9):
    kit.bracket(0.07 + i * 0.1075, 6.62, 0.0, 0.2, 0.4, 0.22, BELT)
for f in (0.2, 0.8):
    kit.box(f, 7.2, 0.7, 2.3, 0.4, 0.05, BELT)                 # apron on the bay front
    kit.box(f, 7.2, 0.75, 1.9, 0.26, 0.03, PANEL)
