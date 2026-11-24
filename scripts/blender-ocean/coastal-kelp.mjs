// Bake continuous stipes and attached gas bladders and blades from the sampled
// donor column. Coordinates arrive in a four-metre plant frame. The game
// scales that frame to the available water column above each holdfast.
export function coastalKelp(leaves, stems) {
  const p = [], i = [], rgb = [], bladeCoord = [];
  const tau = 2 * Math.PI;
  function vertex(x, y, z, pigment, coord = 0) {
    const index = p.length / 3;
    p.push(x / 4, y / 4, z / 4);
    rgb.push(...pigment);
    bladeCoord.push(coord);
    return index;
  }
  function connect(a, b, closed) {
    const spans = closed ? a.length : a.length - 1;
    for (let k = 0; k < spans; k++) {
      const next = (k + 1) % a.length;
      i.push(a[k], b[k], a[next], a[next], b[k], b[next]);
    }
  }
  // The same current carries every attachment along the stipe.
  function axis(x, y, z) {
    const t = y / 4;
    const spread = 0.28 + 0.72 * Math.min(1, t * 3);
    return [spread * x + 1.10 * t * t, y, spread * z + 0.23 * t * Math.sin(4.1 * t)];
  }
  for (const stem of stems) {
    let previous;
    for (const [x, y, z] of stem) {
      const [cx, cy, cz] = axis(x, y, z);
      const radius = 0.022 * (1 - 0.59 * y / 4);
      const ring = Array.from({length: 6}, (_, k) => {
        const angle = tau * k / 6;
        return vertex(cx + radius * Math.cos(angle), cy, cz + radius * Math.sin(angle), [0.20, 0.127, 0.035]);
      });
      if (previous) connect(previous, ring, true);
      previous = ring;
    }
  }
  for (const [x, y, z, length, yaw, tilt, width, , type, seed, phase] of leaves) {
    if (type === 1) continue;
    const [cx, cy, cz] = axis(x, y, z);
    const dx = Math.cos(yaw), dz = Math.sin(yaw);
    const pigment = [0.21 + 0.11 * seed, 0.14 + 0.06 * seed, 0.030 + 0.020 * phase];
    const bud = 0.048 + 0.024 * seed;
    let previous;
    for (const [t, radius] of [[0, 0.004], [0.38, 0.016], [0.65, 0.019], [1, 0.005]]) {
      const ring = Array.from({length: 6}, (_, k) => {
        const a = tau * k / 6;
        const across = radius * Math.cos(a);
        const lift = radius * Math.sin(a);
        return vertex(cx + dx * (bud*t - 0.52*lift) - dz*across,
          cy + 0.60*bud*t + 0.86*lift, cz + dz * (bud*t - 0.52*lift) + dx*across, [0.27, 0.182, 0.054]);
      });
      if (previous) connect(previous, ring, true);
      previous = ring;
    }
    previous = undefined;
    const wave = 4.3 + 2.5 * phase;
    for (let row = 0; row <= 12; row++) {
      const t = row / 12;
      const taper = 0.02 + 0.98 * Math.pow(Math.sin(Math.PI * t ** 0.87), 0.72);
      const reach = bud + length * t * (0.60 + tilt * 0.43);
      const lift = bud * 0.60 + length * (0.23 * Math.sin(t * Math.PI)
        + 0.092 * t * Math.sin(t * wave + seed * tau) - 0.16 * t * t);
      const drift = length * t * t * (0.46 + 0.12 * phase);
      const widthAt = width * 0.40 * taper;
      const twist = (phase - 0.5) * 1.25 + (seed - 0.5) * 2.0 * t
        + 0.28 * Math.sin(t * wave + phase * tau);
      const ring = [-1, -0.36, 0, 0.36, 1].map(side => {
        const margin = Math.abs(side);
        const ripple = Math.sin(t * (12 + 4*seed) + phase*tau + side);
        const cross = side * widthAt * (1 + 0.11 * ripple * margin);
        const cup = widthAt * (0.15 * (1 - margin) + margin * margin * (0.24 + 0.22 * ripple));
        const across = cross * Math.cos(twist) - cup * Math.sin(twist);
        const relief = cross * Math.sin(twist) + cup * Math.cos(twist);
        const tint = (1.0 - 0.19 * margin + 0.15 * t) * (0.94 + 0.06 * ripple * margin);
        // Reserve zero for stipes. Decode in the vertex stage, before interpolation.
        const coord = (Math.round(15*t) << 4) | (1 + Math.round(7*(side+1)));
        return vertex(cx + reach*dx + drift - dz*across, cy + lift + relief,
          cz + reach*dz + dx*across, pigment.map(c => c * tint), coord);
      });
      if (previous) connect(previous, ring, false);
      previous = ring;
    }
  }
  return {p, i, rgb, bladeCoord};
}
