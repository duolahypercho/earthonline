# Ornate arched portal (Eddy corner flats): a white column either side of the entry
# arch, as in the street photo. f = fraction along the front; door spans .595-.735.
E = 0.665
STONE = 'cast-stone-trim@e6e0d0'
for dr in (-1.3, 1.3):
    kit.column(E, 0.3, 3.4, 0.14, 0.17, STONE, dr=dr)        # shaft
    kit.box(E, 0.2, 0.14, 0.42, 0.2, 0.42, STONE, dr=dr)     # base block
    kit.box(E, 3.45, 0.14, 0.44, 0.2, 0.44, STONE, dr=dr)    # capital block
