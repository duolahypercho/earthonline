// Original normalized plant meshes. Bull kelp has one terminal float per stipe
// and a crown of long blades. Eelgrass grows in shoots along buried rhizomes.
// https://farallones.noaa.gov/media/docs/201904-bull-kelp-recovery-plan.pdf
// https://plants.sc.egov.usda.gov/DocumentLibrary/factsheet/pdf/fs_zoma.pdf
const tau = 2 * Math.PI;
function random(seed) {
  return () => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed / 4294967296;
  };
}
function mesh() {
  return {p: [], i: [], rgb: [], bladeCoord: []};
}
function vertex(m, p, pigment, along, across) {
  const index = m.p.length / 3;
  m.p.push(...p);
  m.rgb.push(...pigment);
  m.bladeCoord.push(along == null ? 0 :
    (Math.round(along * 15) << 4) | (1 + Math.round((across + 1) * 7)));
  return index;
}
function join(m, lower, upper, closed) {
  const spans = closed ? lower.length : lower.length - 1;
  for (let i = 0; i < spans; i++) {
    const j = (i + 1) % lower.length;
    m.i.push(lower[i], upper[i], lower[j], lower[j], upper[i], upper[j]);
  }
}
function strip(m, rows, sides, closed, point) {
  let previous;
  for (let row = 0; row <= rows; row++) {
    const ring = [];
    for (let k = 0; k < sides; k++) {
      ring.push(point(row / rows, closed ? tau*k/sides : 2*k/(sides-1)-1));
    }
    if (previous) join(m, previous, ring, closed);
    previous = ring;
  }
}
export function bullKelpBed() {
  const m = mesh(), next = random(491);
  // The two individuals attach separately to the same rock. One reaches the
  // canopy and the younger one streams below it. Each has just one float.
  for (const [ox, oz, h, sway, count] of [[0.06, -0.04, 0.985, 0.18, 32], [-0.055, 0.053, 0.68, 0.11, 15]]) {
    const phase = next()*tau;
    function axis(t) {
      return [ox + sway*t*t + 0.027*Math.sin(5*t+phase)*t,
        h*t, oz + 0.058*Math.sin(4*t+phase)*t];
    }
    strip(m, 24, 8, true, (t, a) => {
      const [x,y,z] = axis(t);
      const radius = 0.0027 + 0.0027*t*t + 0.0019*Math.exp(-30*t);
      return vertex(m,[x+radius*Math.cos(a),y,z+radius*Math.sin(a)],[0.235,0.142,0.046]);
    });
    const [x,y,z] = axis(1);
    const floatRadius = 0.014 + 0.007*h;
    strip(m, 10, 12, true, (t, a) => {
      const r = floatRadius*(0.025 + 0.975*Math.pow(Math.sin(Math.PI*t),0.72));
      return vertex(m,[x+0.022*t+r*Math.cos(a),y+0.044*(t-0.24),z+r*Math.sin(a)],[0.295,0.186,0.056]);
    });
    for (let k=0;k<count;k++) {
      const angle = k*2.399963 + (next()-0.5)*0.66;
      const dx = Math.cos(angle), dz = Math.sin(angle);
      const length = (0.24 + 0.48*next())*h;
      const width = (0.007 + 0.0065*next())*h;
      const curtain = next() < 0.38;
      const hang = curtain ? 0.49+0.34*next() : 0.04+0.28*next();
      const ripple = next()*tau, pitch = 7+next()*6;
      const age = next(), torn = next() < 0.30;
      const shoulder = (k%2 ? 1 : -1)*floatRadius*0.58;
      const baseX = x+0.014+floatRadius*0.44*Math.sin(angle);
      const baseZ = z+shoulder;
      const widthPhase = next()*tau;
      strip(m, 29, 5, false, (t, side) => {
        const reach = length*t*((curtain ? 0.46 : 0.65) - 0.20*t);
        const flutter = 0.041*Math.sin(pitch*t+ripple) + 0.019*Math.sin(1.8*pitch*t+widthPhase);
        const drape = length*(t*flutter - hang*t**1.50);
        const drift = length*(curtain ? 0.27 : 0.61)*t*t;
        const lateral = length*(0.082*Math.sin(pitch*0.45*t+ripple)+0.021*Math.sin(8.8*t+widthPhase))*t;
        // A long, nearly parallel-sided tape opens after the narrow base.
        // Old blades retain a truncated end; the opposite margin stays intact.
        const tip = torn ? 1-0.48*t**5 : 1-0.94*t**3;
        const spread = (0.035+0.965*(1-Math.exp(-12*t)))*tip;
        const margin = Math.abs(side), wave = Math.sin(pitch*t+ripple+side);
        const lobe = 1+0.20*Math.sin(19*t+widthPhase+side)+0.09*Math.sin(31*t+side*1.3+ripple);
        const edge = side*width*spread*(1+margin*(lobe-1));
        const fold = 0.71*Math.sin(7.1*t+ripple)+0.78*(age-0.5)+0.83*t*dx;
        const roll = edge*Math.cos(fold);
        const lift = edge*Math.sin(fold) + width*spread*margin*margin*(0.20+0.42*wave);
        const stain = 1-0.19*age-0.11*margin+0.06*wave*t;
        return vertex(m,[baseX+dx*reach+drift-dz*(roll+lateral),
          y+0.019+drape+lift,baseZ+dz*reach+dx*(roll+lateral)],
          [0.275*stain, (0.164+0.031*age)*stain,0.045*stain],t,side);
      });
    }
  }
  return m;
}
export function eelgrassBed() {
  const m = mesh(), next = random(283);
  for (let shoot=0;shoot<8;shoot++) {
    const a=shoot*2.399963, r=shoot ? 0.12+0.28*next() : 0;
    const ox=r*Math.cos(a), oz=r*Math.sin(a), height=0.57+0.42*next();
    const count=3+Math.floor(next()*3);
    for (let k=0;k<count;k++) {
      const az=a+k*2.399963+next()*0.38, dx=Math.cos(az), dz=Math.sin(az);
      const h=height*(0.52+0.48*next()), lean=0.16+0.44*next(), bend=0.23+0.36*next();
      const width=0.006+0.006*next(), age=next(), phase=next()*tau;
      strip(m, 12, 3, false, (t, side) => {
        const reach=h*lean*t**1.6;
        const arc=h*(t-bend*t**2.7);
        const oxs=0.014*t*Math.sin(t*6+phase);
        // The blade keeps its width most of its length and ends in a rounded tip.
        const edge=side*width*(0.55+0.45*Math.sin(Math.PI*t*0.63))*(1-0.80*t**24);
        const twist=0.50*Math.sin(t*3.8+phase);
        const shade=0.90+0.10*(1-Math.abs(side));
        const dry=age**3*t**4;
        return vertex(m,[ox+dx*reach+0.28*h*t*t-dz*(edge*Math.cos(twist)+oxs),
          arc+edge*Math.sin(twist),oz+dz*reach+dx*(edge*Math.cos(twist)+oxs)],
          [(0.062+0.042*age+0.105*dry)*shade,(0.125+0.10*(1-age)-0.028*dry)*shade,
           (0.030+0.019*age)*shade],t,side);
      });
    }
  }
  return m;
}
