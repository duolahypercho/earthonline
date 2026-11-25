// Intersect joint planes to form submerged bedrock with a few freshly exposed
// fracture faces. Several overlapping ribs make one continuous ledge.
function stone(mesh, seed, centre, extent, yaw, tint) {
  const cuts = [
    [1,0.13,0.15,0.85], [-1,0.18,-0.18,0.90], [0.28,1,0.12,0.72],
    [-0.25,0.95,0.32,0.73], [-0.21,-1,0.16,0.75], [0.16,0.18,1,0.83],
    [-0.18,0.18,-1,0.91], [0.69,0.61,0.51,0.96], [-0.81,0.43,0.56,0.96],
    [0.48,0.65,-0.63,0.92], [-0.60,0.52,-0.65,0.97], [0.7,-0.56,0.51,0.98],
    [-0.65,-0.46,-0.63,0.94],
  ];
  let state=seed;
  function random() { state=(Math.imul(1664525,state)+1013904223)>>>0; return state/4294967296; }
  const dot=(a,b)=>a[0]*b[0]+a[1]*b[1]+a[2]*b[2];
  const cross=(a,b)=>[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]];
  const unit=v=>v.map(x=>x/Math.hypot(...v));
  const planes=cuts.map(([x,y,z,d])=>[unit([x+0.21*(random()-0.5),y,z+0.23*(random()-0.5)]),d*(0.82+0.22*random())]);
  const positions=[], faces=[], welded=new Map();
  function corner(p) {
    const key=p.map(x=>Math.round(x*1e5)).join(',');
    if (!welded.has(key)) {welded.set(key,positions.length);positions.push(p);}
    return welded.get(key);
  }
  for(const [n,d] of planes){
    const u=unit(cross(n,Math.abs(n[1])<0.8?[0,1,0]:[1,0,0])), v=cross(n,u);
    let poly=[[-3,-3],[3,-3],[3,3],[-3,3]].map(([a,b])=>n.map((x,i)=>x*d+a*u[i]+b*v[i]));
    for(const [q,k] of planes){
      if(q===n)continue;
      const clipped=[];
      for(let i=0;i<poly.length;i++){
        const a=poly[i], b=poly[(i+1)%poly.length], da=dot(q,a)-k, db=dot(q,b)-k;
        if(da<=0)clipped.push(a);
        if((da<0)!==(db<0))clipped.push(a.map((x,j)=>x+(b[j]-x)*da/(da-db)));
      }
      poly=clipped;
    }
    if(poly.length<3)continue;
    faces.push(poly.map(corner));
  }
  // Two subdivision steps wear exposed corners while retaining the larger
  // cleavage planes. Shared vertices give the bevels continuous normals.
  const mean=ps=>[0,1,2].map(i=>ps.reduce((sum,p)=>sum+p[i],0)/ps.length);
  const mix=(a,b,t)=>a.map((v,i)=>v*(1-t)+b[i]*t);
  function roundHull(points,polygons,wear) {
    const centres=polygons.map(f=>mean(f.map(i=>points[i])));
    const byVertex=points.map(()=>({faces:[],midpoints:[]}));
    const edges=new Map();
    const edgeKey=(a,b)=>a<b ? a+','+b : b+','+a;
    for (const [f,face] of polygons.entries()) for (let i=0;i<face.length;i++) {
      const a=face[i], b=face[(i+1)%face.length], key=edgeKey(a,b);
      byVertex[a].faces.push(centres[f]);
      if(!edges.has(key)){
        const mid=mean([points[a],points[b]]);
        edges.set(key,{a,b,mid,faces:[]});
        byVertex[a].midpoints.push(mid);byVertex[b].midpoints.push(mid);
      }
      edges.get(key).faces.push(centres[f]);
    }
    const next=points.map((p,i)=>{
      const {faces,midpoints}=byVertex[i], n=faces.length;
      const f=mean(faces), r=mean(midpoints);
      return mix(p,p.map((v,j)=>(f[j]+2*r[j]+(n-3)*v)/n),wear);
    });
    for(const e of edges.values()){
      const mid=mean([points[e.a],points[e.b],...e.faces]);
      e.index=next.push(mix(e.mid,mid,wear))-1;
    }
    const nextFaces=[];
    for(const [f,face] of polygons.entries()){
      const c=next.push(centres[f])-1;
      face.forEach((v,i)=>nextFaces.push([v,edges.get(edgeKey(v,face[(i+1)%face.length])).index,
        c,edges.get(edgeKey(v,face[(i+face.length-1)%face.length])).index]));
    }
    return [next,nextFaces];
  }
  let [points,polys]=roundHull(positions,faces,0.78);
  [points,polys]=roundHull(points,polys,0.90);
  [points,polys]=roundHull(points,polys,0.96);
  const first=mesh.p.length/3, phase=random()*6.28;
  for (const [x,y,z] of points){
    const rough=1+0.055*Math.sin(8*x+2.9*y+phase)*Math.sin(6*z-2*y)+0.028*Math.sin(16*z+7*x+phase);
    const sx=x*extent[0]*rough,sz=z*extent[2]*rough;
    mesh.p.push(centre[0]+Math.cos(yaw)*sx-Math.sin(yaw)*sz,centre[1]+y*extent[1]*rough,
      centre[2]+Math.sin(yaw)*sx+Math.cos(yaw)*sz);
    const grain=0.86+0.13*Math.sin(5*x+2*z+phase)*Math.cos(4*y+3*z);
    mesh.rgb.push(...tint.map(c=>c*grain*(0.91+0.09*y)));
  }
  for(const face of polys)for(let i=1;i+1<face.length;i++)mesh.i.push(first+face[0],first+face[i],first+face[i+1]);
}
export function bedrockLedge(){
  const m={p:[],i:[],rgb:[]};
  stone(m,71,[0,0.10,0],[1.03,0.64,0.53],0.14,[0.20,0.185,0.157]);
  stone(m,53,[-0.17,-0.01,0.45],[0.76,0.48,0.25],0.11,[0.16,0.17,0.15]);
  stone(m,89,[0.28,-0.03,-0.41],[0.61,0.39,0.24],0.04,[0.20,0.18,0.16]);
  return m;
}
export function fracturedBoulder(){
  const m={p:[],i:[],rgb:[]};
  stone(m,117,[0,0,0],[0.83,0.82,0.73],0.64,[0.185,0.206,0.189]);
  return m;
}
