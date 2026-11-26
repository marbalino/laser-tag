// Creek Laser Tag — the browser side. Three.js world, your own movement, and the picture of everyone else from the
// server's snapshots (drawn 100 ms behind, smoothly). The server decides who gets tagged.
import * as THREE from 'three';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';

const $ = id => document.getElementById(id);
const PALETTE = ['#ff3df2', '#22f0ff', '#ffe23d', '#7dff3d', '#ff8a1f', '#a66bff', '#ff4d6d', '#3dffb4'];
const POWER = { rapid: { name: 'Rapid', color: '#ffe23d', bit: 2 }, over: { name: 'Overcharge', color: '#ff4d6d', bit: 4 }, speed: { name: 'Speed', color: '#7dff3d', bit: 8 }, drone: { name: 'Drone', color: '#b48cff', bit: 0 } };
const EYE = 5.4, GRAV = 64, JUMP1 = 22, JUMP2 = 20, INTERP = 0.1;
const store = { get(k, d) { try { const v = localStorage.getItem('lt.' + k); return v == null ? d : JSON.parse(v); } catch (e) { return d; } }, set(k, v) { try { localStorage.setItem('lt.' + k, JSON.stringify(v)); } catch (e) { /* private mode */ } } };
// phones and tablets: on from the start when the device says it is touch-first, and in any case from the first touch
let touchMode = (matchMedia('(pointer: coarse)').matches || navigator.maxTouchPoints > 1) && ('ontouchstart' in window || navigator.maxTouchPoints > 0);

/* ------------------------------------------------------------------ terrain */
const meta = await (await fetch('./terrain.json')).json();
const raw = new Uint8Array(await (await fetch('./terrain.bin')).arrayBuffer());
const N = meta.n, CELL = meta.cell, SIZE = meta.size;
const H = new Float32Array(N * N), WET = new Uint8Array(N * N);
{ const dv = new DataView(raw.buffer); for (let i = 0; i < N * N; i++) H[i] = dv.getUint16(i * 2, true) / 100;
  const off = N * N * 2; for (let i = 0; i < N * N; i++) WET[i] = (raw[off + (i >> 3)] >> (7 - (i & 7))) & 1; }
function ground(x, z) {
  const fx = Math.min(Math.max(x / CELL, 0), N - 1.001), fz = Math.min(Math.max(z / CELL, 0), N - 1.001);
  const i = fx | 0, j = fz | 0, u = fx - i, v = fz - j;
  const a = H[j * N + i], b = H[j * N + i + 1], c = H[(j + 1) * N + i], d = H[(j + 1) * N + i + 1];
  return (a * (1 - u) + b * u) * (1 - v) + (c * (1 - u) + d * u) * v;
}
const wet = (x, z) => WET[Math.min(Math.max(Math.round(z / CELL), 0), N - 1) * N + Math.min(Math.max(Math.round(x / CELL), 0), N - 1)];
let PILLARS = [], BOXES = [];
// wall pieces: boxes turned about the vertical (local = turned back from world)
function setBoxes(list) { BOXES = list.map(([cx, cy, cz, hx, hy, hz, a, tint]) => ({ cx, cy, cz, hx, hy, hz, c: Math.cos(a), s: Math.sin(a), top: cy + hy, bot: cy - hy, r2: (hx + hz + 3) ** 2, tint })); }
function pushBoxes(x, z, y) {
  for (const b of BOXES) {
    if (y >= b.top - 0.35 || y + 5.8 <= b.bot) continue;
    const px = x - b.cx, pz = z - b.cz; if (px * px + pz * pz > b.r2) continue;
    let lx = b.c * px - b.s * pz, lz = b.s * px + b.c * pz;
    const qx = Math.min(Math.max(lx, -b.hx), b.hx), qz = Math.min(Math.max(lz, -b.hz), b.hz), ex = lx - qx, ez = lz - qz, d = Math.hypot(ex, ez), R = 1.4;
    if (d >= R) continue;
    if (d > 1e-4) { lx = qx + ex / d * R; lz = qz + ez / d * R; }
    else if (b.hx - Math.abs(lx) < b.hz - Math.abs(lz)) lx = Math.sign(lx || 1) * (b.hx + R); else lz = Math.sign(lz || 1) * (b.hz + R);
    x = b.cx + b.c * lx + b.s * lz; z = b.cz - b.s * lx + b.c * lz;
  }
  return [x, z];
}
function rayBoxes(o, d, tmax) {
  let best = null;
  for (const b of BOXES) {
    const px = o[0] - b.cx, py = o[1] - b.cy, pz = o[2] - b.cz;
    if (px * px + pz * pz > (tmax + b.hx + b.hz) ** 2) continue;
    const lx = b.c * px - b.s * pz, lz = b.s * px + b.c * pz, dx = b.c * d[0] - b.s * d[2], dz = b.s * d[0] + b.c * d[2];
    let tn = -1e18, tf = 1e18, ok = true;
    for (const [p, dd, h] of [[lx, dx, b.hx], [py, d[1], b.hy], [lz, dz, b.hz]]) {
      if (Math.abs(dd) < 1e-9) { if (Math.abs(p) > h) { ok = false; break; } continue; }
      let t1 = (-h - p) / dd, t2 = (h - p) / dd; if (t1 > t2) [t1, t2] = [t2, t1];
      tn = Math.max(tn, t1); tf = Math.min(tf, t2); if (tn > tf) { ok = false; break; }
    }
    if (!ok || tf < 0) continue;
    const t = Math.max(tn, 0); if (t < tmax && (best == null || t < best)) best = t;
  }
  return best;
}
function raySolid(o, d, tmax) { const a = rayPillars(o, d, tmax), b = rayBoxes(o, d, a ?? tmax); return b ?? a; }
function pushOut(x, z, y) {
  for (const [px, pz, r, , top] of PILLARS) {
    if (y >= top - 0.5) continue;
    let dx = x - px, dz = z - pz, d = Math.hypot(dx, dz); const lim = r + 1.6;
    if (d < lim) { if (d < 1e-3) { dx = 1; dz = 0; d = 1; } x = px + dx / d * lim; z = pz + dz / d * lim; }
  }
  [x, z] = pushBoxes(x, z, y);
  return [Math.min(Math.max(x, 2), SIZE - 2), Math.min(Math.max(z, 2), SIZE - 2)];
}
function floorAt(x, z, y) {
  let f = ground(x, z);
  for (const [px, pz, r, , top] of PILLARS) if (y >= top - 1 && Math.hypot(x - px, z - pz) <= r) f = Math.max(f, top);
  for (const b of BOXES) {
    if (y < b.top - 1 || b.top <= f) continue;
    const px = x - b.cx, pz = z - b.cz; if (px * px + pz * pz > b.r2) continue;
    if (Math.abs(b.c * px - b.s * pz) <= b.hx + 0.3 && Math.abs(b.s * px + b.c * pz) <= b.hz + 0.3) f = b.top;
  }
  return f;
}
function rayGround(o, d, tmax) {
  for (let t = 2; t < tmax; t += 2) {
    const x = o[0] + d[0] * t, y = o[1] + d[1] * t, z = o[2] + d[2] * t;
    if (x < 0 || z < 0 || x > SIZE || z > SIZE) return null;
    if (y < ground(x, z)) { let lo = t - 2, hi = t; for (let k = 0; k < 6; k++) { const m = (lo + hi) / 2; if (o[1] + d[1] * m < ground(o[0] + d[0] * m, o[2] + d[2] * m)) hi = m; else lo = m; } return hi; }
  }
  return null;
}
function rayPillars(o, d, tmax) {
  let best = null;
  for (const [x, z, r, b, top] of PILLARS) {
    const ox = o[0] - x, oz = o[2] - z, a = d[0] * d[0] + d[2] * d[2];
    if (a > 1e-9) {
      const bb = 2 * (ox * d[0] + oz * d[2]), c = ox * ox + oz * oz - r * r, disc = bb * bb - 4 * a * c;
      if (disc >= 0) { const s = Math.sqrt(disc); for (const t of [(-bb - s) / (2 * a), (-bb + s) / (2 * a)]) { if (t > 0 && t < tmax && (best == null || t < best)) { const y = o[1] + d[1] * t; if (y >= b && y <= top) { best = t; break; } } } }
    }
    if (Math.abs(d[1]) > 1e-6) { const t = (top - o[1]) / d[1]; if (t > 0 && t < tmax && (best == null || t < best) && Math.hypot(o[0] + d[0] * t - x, o[2] + d[2] * t - z) <= r) best = t; }
  }
  return best;
}

/* ------------------------------------------------------------------ renderer and scene */
const canvas = $('view');
const renderer = new THREE.WebGLRenderer({ canvas, antialias: !touchMode, powerPreference: 'high-performance' });
renderer.toneMapping = THREE.ACESFilmicToneMapping;
renderer.toneMappingExposure = 1.05;
const scene = new THREE.Scene();
const FOG = new THREE.Color(0x0d0620);
scene.background = FOG;
scene.fog = new THREE.FogExp2(FOG, 0.0011);
const camera = new THREE.PerspectiveCamera(75, 1, 0.3, 5000);
camera.rotation.order = 'YXZ';
scene.add(camera);
scene.add(new THREE.HemisphereLight(0x6a5cff, 0x120820, 1.1));
const moon = new THREE.DirectionalLight(0xbfd8ff, 1.3); moon.position.set(-400, 800, 300); scene.add(moon);

let glowOn = store.get('glow', !touchMode);
const composer = new EffectComposer(renderer);
composer.addPass(new RenderPass(scene, camera));
const bloom = new UnrealBloomPass(new THREE.Vector2(256, 256), 0.55, 0.35, 0.85);
bloom.enabled = glowOn;
composer.addPass(bloom);
composer.addPass(new OutputPass());
function resize() {
  const w = innerWidth, h = innerHeight, pr = Math.min(devicePixelRatio, glowOn ? 1.5 : 1.25);
  renderer.setPixelRatio(pr); renderer.setSize(w, h, false); composer.setPixelRatio(pr); composer.setSize(w, h);
  bloom.resolution.set(w / 2, h / 2);
  camera.aspect = w / h; camera.updateProjectionMatrix();
}
addEventListener('resize', resize); resize();

// sky: a gradient dome and stars
{
  const sky = new THREE.Mesh(new THREE.SphereGeometry(3000, 32, 16), new THREE.ShaderMaterial({
    side: THREE.BackSide, depthWrite: false, fog: false,
    vertexShader: 'varying vec3 vD; void main(){ vD = normalize(position); gl_Position = projectionMatrix*modelViewMatrix*vec4(position,1.); }',
    fragmentShader: `varying vec3 vD; void main(){ float h = clamp(vD.y,-0.2,1.); vec3 hor = vec3(0.32,0.06,0.38), zen = vec3(0.01,0.01,0.05);
      vec3 c = mix(hor, zen, smoothstep(-0.02, 0.45, h)); c += vec3(1.0,0.25,0.6)*0.35*exp(-abs(h-0.0)*18.); gl_FragColor = vec4(c,1.); }`,
  }));
  sky.renderOrder = -10; sky.name = 'sky'; scene.add(sky);
  const g = new THREE.BufferGeometry(), pts = [];
  for (let i = 0; i < 1400; i++) { const u = Math.random() * 2 - 1, a = Math.random() * Math.PI * 2, y = Math.abs(u) * 0.95 + 0.05, r = Math.sqrt(1 - y * y); pts.push(Math.cos(a) * r * 2800, y * 2800, Math.sin(a) * r * 2800); }
  g.setAttribute('position', new THREE.Float32BufferAttribute(pts, 3));
  const stars = new THREE.Points(g, new THREE.PointsMaterial({ color: 0xffffff, size: 1.6, sizeAttenuation: false, fog: false, transparent: true, opacity: .9 }));
  stars.name = 'stars'; scene.add(stars);
}

// the ground: height colours, a neon grid, contour lines, and the creek glowing
const terrainU = { uTime: { value: 0 }, uCam: { value: new THREE.Vector3() } };
{
  const g = new THREE.BufferGeometry(), pos = new Float32Array(N * N * 3), w = new Float32Array(N * N);
  for (let j = 0; j < N; j++) for (let i = 0; i < N; i++) { const k = j * N + i; pos[k * 3] = i * CELL; pos[k * 3 + 1] = H[k]; pos[k * 3 + 2] = j * CELL; w[k] = WET[k]; }
  const idx = new Uint32Array((N - 1) * (N - 1) * 6); let q = 0;
  for (let j = 0; j < N - 1; j++) for (let i = 0; i < N - 1; i++) { const a = j * N + i, b = a + 1, c = a + N, d = c + 1; idx[q++] = a; idx[q++] = c; idx[q++] = b; idx[q++] = b; idx[q++] = c; idx[q++] = d; }
  g.setAttribute('position', new THREE.BufferAttribute(pos, 3)); g.setAttribute('wet', new THREE.BufferAttribute(w, 1)); g.setIndex(new THREE.BufferAttribute(idx, 1));
  g.computeVertexNormals();
  const m = new THREE.ShaderMaterial({
    uniforms: terrainU,
    vertexShader: `attribute float wet; varying vec3 vP; varying vec3 vN; varying float vW;
      void main(){ vP = position; vN = normal; vW = wet; gl_Position = projectionMatrix*modelViewMatrix*vec4(position,1.); }`,
    fragmentShader: `uniform float uTime; uniform vec3 uCam; varying vec3 vP; varying vec3 vN; varying float vW;
      float line(float v, float step){ float f = abs(fract(v/step+0.5)-0.5)*step; float w = fwidth(v); return 1.0 - smoothstep(0.4*w, 1.4*w, f); }
      void main(){
        vec3 n = normalize(vN);
        float hh = smoothstep(0.0, 140.0, vP.y);
        vec3 col = mix(vec3(0.020,0.035,0.090), vec3(0.16,0.05,0.21), hh);
        float ndl = max(dot(n, normalize(vec3(-0.4,0.8,0.3))), 0.0);
        col *= 0.35 + 1.0*ndl;
        float dist = length(vP - uCam);
        float fade = exp(-dist/520.0);
        float g = max(line(vP.x, 25.0), line(vP.z, 25.0));
        col += vec3(0.10,0.85,1.0) * g * 0.55 * fade;
        float c1 = line(vP.y, 2.0), c5 = line(vP.y, 10.0);
        col += vec3(0.95,0.20,1.0) * (c1*0.22 + c5*0.6) * (0.35+0.65*fade);
        float rip = 0.5 + 0.5*sin(uTime*1.8 + vP.x*0.12 + vP.z*0.09) * sin(uTime*1.1 - vP.z*0.07);
        vec3 water = vec3(0.0,0.10,0.20) + vec3(0.10,0.55,0.75)*(0.25 + 0.35*rip);
        col = mix(col, water, smoothstep(0.25, 0.9, vW));
        float fogf = 1.0 - exp(-dist*0.0011);
        col = mix(col, vec3(0.05,0.024,0.125), fogf);
        gl_FragColor = vec4(col, 1.0);
      }`,
  });
  const mesh = new THREE.Mesh(g, m); mesh.name = 'ground'; scene.add(mesh);
}

// the laser fence round the arena
{
  const S = 160, pos = [], uv = [], idx = [];
  const edge = [];
  for (let k = 0; k < S; k++) edge.push([k / S * SIZE, 0]);
  for (let k = 0; k < S; k++) edge.push([SIZE, k / S * SIZE]);
  for (let k = 0; k < S; k++) edge.push([SIZE - k / S * SIZE, SIZE]);
  for (let k = 0; k <= S; k++) edge.push([0, SIZE - k / S * SIZE]);
  edge.forEach(([x, z], i) => { const g = ground(x, z); pos.push(x, g - 2, z, x, g + 30, z); uv.push(i, 0, i, 1); if (i) { const a = (i - 1) * 2; idx.push(a, a + 1, a + 2, a + 1, a + 3, a + 2); } });
  const g = new THREE.BufferGeometry(); g.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3)); g.setAttribute('uv', new THREE.Float32BufferAttribute(uv, 2)); g.setIndex(idx);
  const m = new THREE.ShaderMaterial({ uniforms: terrainU, transparent: true, depthWrite: false, side: THREE.DoubleSide, blending: THREE.AdditiveBlending,
    vertexShader: 'varying vec2 vU; void main(){ vU = uv; gl_Position = projectionMatrix*modelViewMatrix*vec4(position,1.); }',
    fragmentShader: `uniform float uTime; varying vec2 vU; void main(){ float a = (1.0 - vU.y); a = a*a*0.55;
      float s = smoothstep(0.85, 1.0, abs(sin((vU.y*6.0 - uTime*0.6)*3.14159))) * 0.6;
      gl_FragColor = vec4(vec3(1.0,0.24,0.95)*(a + s*(1.0-vU.y)), 1.0); }` });
  scene.add(new THREE.Mesh(g, m));
}

/* ------------------------------------------------------------------ pillars and power-ups (built on welcome) */
const arena = new THREE.Group(); scene.add(arena);
const spotMeshes = [];
function buildArena(pillars, spots) {
  arena.clear(); spotMeshes.length = 0; PILLARS = pillars;
  const body = new THREE.MeshStandardMaterial({ color: 0x141026, roughness: .55, metalness: .4 });
  pillars.forEach(([x, z, r, b, top], i) => {
    const h = top - b, c = new THREE.Color(PALETTE[i % PALETTE.length]);
    const m = new THREE.Mesh(new THREE.CylinderGeometry(r, r, h, 20), body); m.position.set(x, b + h / 2, z); arena.add(m);
    const ringM = new THREE.MeshBasicMaterial({ color: c.clone().multiplyScalar(1.8) });
    for (const y of [top - 0.6, b + (top - b) * 0.55]) { const ring = new THREE.Mesh(new THREE.CylinderGeometry(r + 0.08, r + 0.08, 0.5, 20, 1, true), ringM); ring.position.set(x, y, z); arena.add(ring); }
    const cap = new THREE.Mesh(new THREE.CircleGeometry(r * 0.75, 20), new THREE.MeshBasicMaterial({ color: c.clone().multiplyScalar(0.8) })); cap.rotation.x = -Math.PI / 2; cap.position.set(x, top + 0.02, z); arena.add(cap);
  });
  for (const s of spots) {
    const c = new THREE.Color(POWER[s.kind].color);
    const grp = new THREE.Group(); grp.position.set(s.x, s.y, s.z);
    const gem = new THREE.Mesh(new THREE.OctahedronGeometry(1.6), new THREE.MeshBasicMaterial({ color: c.clone().multiplyScalar(2.5) })); gem.position.y = 3.2; grp.add(gem);
    const beam = new THREE.Mesh(new THREE.CylinderGeometry(0.35, 0.35, 120, 8, 1, true), new THREE.MeshBasicMaterial({ color: c, transparent: true, opacity: .25, blending: THREE.AdditiveBlending, depthWrite: false })); beam.position.y = 60; grp.add(beam);
    const pad = new THREE.Mesh(new THREE.RingGeometry(2.6, 3.4, 32), new THREE.MeshBasicMaterial({ color: c.clone().multiplyScalar(1.6), side: THREE.DoubleSide })); pad.rotation.x = -Math.PI / 2; pad.position.y = 0.25; grp.add(pad);
    arena.add(grp); spotMeshes[s.i] = { grp, gem, beam, s };
    setSpot(s.i, s.up);
  }
}
const TINTS = ['#ff3df2', '#22f0ff', '#ffe23d', '#7dff3d', '#ff8a1f', '#a66bff', '#ff4d6d', '#3dffb4'];
function buildRuins(boxes, windows) {
  setBoxes(boxes);
  // every box face, merged into one mesh; each face knows where its edges are, so the shader can light them
  const P = [], Nn = [], U = [], S = [], C = [], I = [];
  const faces = [[[1, 0, 0], [0, 0, -1], [0, 1, 0]], [[-1, 0, 0], [0, 0, 1], [0, 1, 0]], [[0, 1, 0], [1, 0, 0], [0, 0, -1]], [[0, -1, 0], [1, 0, 0], [0, 0, 1]], [[0, 0, 1], [1, 0, 0], [0, 1, 0]], [[0, 0, -1], [-1, 0, 0], [0, 1, 0]]];
  for (const [cx, cy, cz, hx, hy, hz, a, tint] of boxes) {
    const c = Math.cos(a), sn = Math.sin(a), h = [hx, hy, hz], col = new THREE.Color(TINTS[tint % TINTS.length]);
    const W = ([x, y, z]) => [cx + c * x + sn * z, cy + y, cz - sn * x + c * z];
    for (const [n, u, v] of faces) {
      const ext = k => Math.abs(u[k]) * h[k] + 0, su = 2 * (Math.abs(u[0]) * hx + Math.abs(u[1]) * hy + Math.abs(u[2]) * hz), sv = 2 * (Math.abs(v[0]) * hx + Math.abs(v[1]) * hy + Math.abs(v[2]) * hz);
      const base = P.length / 3;
      for (const [a1, b1] of [[-1, -1], [1, -1], [1, 1], [-1, 1]]) {
        const l = [0, 1, 2].map(k => n[k] * h[k] + u[k] * a1 * su / 2 + v[k] * b1 * sv / 2);
        P.push(...W(l)); Nn.push(c * n[0] + sn * n[2], n[1], -sn * n[0] + c * n[2]);
        U.push((a1 + 1) / 2 * su, (b1 + 1) / 2 * sv); S.push(su, sv); C.push(col.r, col.g, col.b);
      }
      I.push(base, base + 1, base + 2, base, base + 2, base + 3);
    }
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.Float32BufferAttribute(P, 3)); g.setAttribute('normal', new THREE.Float32BufferAttribute(Nn, 3));
  g.setAttribute('fuv', new THREE.Float32BufferAttribute(U, 2)); g.setAttribute('fsz', new THREE.Float32BufferAttribute(S, 2)); g.setAttribute('tint', new THREE.Float32BufferAttribute(C, 3)); g.setIndex(I);
  const m = new THREE.ShaderMaterial({ uniforms: terrainU,
    vertexShader: `attribute vec2 fuv; attribute vec2 fsz; attribute vec3 tint; varying vec2 vU; varying vec2 vS; varying vec3 vT; varying vec3 vN; varying vec3 vP;
      void main(){ vU = fuv; vS = fsz; vT = tint; vN = normal; vP = position; gl_Position = projectionMatrix*modelViewMatrix*vec4(position,1.); }`,
    fragmentShader: `uniform vec3 uCam; varying vec2 vU; varying vec2 vS; varying vec3 vT; varying vec3 vN; varying vec3 vP;
      float h(vec2 p){ return fract(sin(dot(p, vec2(12.9898,78.233)))*43758.5453); }
      void main(){
        vec3 n = normalize(vN); float ndl = max(dot(n, normalize(vec3(-0.4,0.8,0.3))), 0.0);
        vec2 b = floor(vU * vec2(0.5, 1.0)); float brick = 0.85 + 0.3*h(b);
        vec3 col = vec3(0.13,0.11,0.19) * brick * (0.45 + 0.9*ndl);
        float d = min(min(vU.x, vS.x - vU.x), min(vU.y, vS.y - vU.y));
        col += vT * (1.0 - smoothstep(0.05, 0.22, d)) * 1.3;
        float dist = length(vP - uCam); col = mix(col, vec3(0.05,0.024,0.125), 1.0 - exp(-dist*0.0011));
        gl_FragColor = vec4(col, 1.0); }` });
  const mesh = new THREE.Mesh(g, m); mesh.name = 'ruins'; arena.add(mesh);
  // broken glass: jagged shards hanging into each window from its edges
  const G2 = [];
  windows.forEach(([cx, cy, cz, hw, hh, a], wi) => {
    let seed = wi * 9301 + 49297; const rnd = () => (seed = (seed * 233280 + 49297) % 233280) / 233280;
    const ax = [Math.cos(a), 0, -Math.sin(a)], pt = (u, v) => [cx + ax[0] * u, cy + v, cz + ax[2] * u];
    const n = 3 + Math.floor(rnd() * 4);
    for (let k = 0; k < n; k++) {
      const side = Math.floor(rnd() * 4), f0 = rnd() * 0.7, f1 = f0 + 0.15 + rnd() * 0.3, reach = 0.25 + rnd() * 0.5, tipf = f0 + (f1 - f0) * rnd();
      let A, B, T;
      if (side === 0) { A = pt(-hw + 2 * hw * f0, hh); B = pt(-hw + 2 * hw * Math.min(f1, 1), hh); T = pt(-hw + 2 * hw * tipf, hh - 2 * hh * reach); }
      else if (side === 1) { A = pt(-hw + 2 * hw * f0, -hh); B = pt(-hw + 2 * hw * Math.min(f1, 1), -hh); T = pt(-hw + 2 * hw * tipf, -hh + 2 * hh * reach * 0.6); }
      else if (side === 2) { A = pt(-hw, -hh + 2 * hh * f0); B = pt(-hw, -hh + 2 * hh * Math.min(f1, 1)); T = pt(-hw + 2 * hw * reach * 0.7, -hh + 2 * hh * tipf); }
      else { A = pt(hw, -hh + 2 * hh * f0); B = pt(hw, -hh + 2 * hh * Math.min(f1, 1)); T = pt(hw - 2 * hw * reach * 0.7, -hh + 2 * hh * tipf); }
      G2.push(...A, ...B, ...T);
    }
  });
  const gg = new THREE.BufferGeometry(); gg.setAttribute('position', new THREE.Float32BufferAttribute(G2, 3));
  arena.add(new THREE.Mesh(gg, new THREE.MeshBasicMaterial({ color: new THREE.Color(0x7fe9ff).multiplyScalar(0.7), transparent: true, opacity: 0.38, side: THREE.DoubleSide, blending: THREE.AdditiveBlending, depthWrite: false })));
}
function setSpot(i, up) { const m = spotMeshes[i]; if (!m) return; m.s.up = up; m.gem.visible = up; m.beam.visible = up; }

/* ------------------------------------------------------------------ people */
function label(text, color) {
  const c = document.createElement('canvas'); c.width = 256; c.height = 64; const x = c.getContext('2d');
  x.font = 'bold 34px Trebuchet MS, sans-serif'; x.textAlign = 'center'; x.textBaseline = 'middle';
  x.shadowColor = color; x.shadowBlur = 12; x.fillStyle = '#fff'; x.fillText(text, 128, 32);
  const t = new THREE.CanvasTexture(c); t.colorSpace = THREE.SRGBColorSpace;
  const s = new THREE.Sprite(new THREE.SpriteMaterial({ map: t, depthWrite: false, transparent: true, fog: false })); s.scale.set(9, 2.25, 1); return s;
}
function makeAvatar(color, name) {
  const c = new THREE.Color(color), bright = c.clone().multiplyScalar(1.7);
  const g = new THREE.Group();
  const suit = new THREE.MeshStandardMaterial({ color: 0x1a1830, roughness: .5, metalness: .35, emissive: c, emissiveIntensity: .18 });
  const glow = new THREE.MeshBasicMaterial({ color: bright });
  const torso = new THREE.Mesh(new THREE.CapsuleGeometry(1.15, 2.4, 4, 12), suit); torso.position.y = 2.6; g.add(torso);
  for (const y of [2.0, 3.3]) { const r = new THREE.Mesh(new THREE.TorusGeometry(1.2, 0.11, 6, 24), glow); r.rotation.x = Math.PI / 2; r.position.y = y; g.add(r); }
  const upper = new THREE.Group(); upper.position.y = 4.4; g.add(upper);
  const head = new THREE.Mesh(new THREE.SphereGeometry(0.85, 16, 12), suit); head.position.y = 1.0; upper.add(head);
  const visor = new THREE.Mesh(new THREE.BoxGeometry(1.25, 0.32, 0.5), glow); visor.position.set(0, 1.05, -0.62); upper.add(visor);
  const gun = new THREE.Mesh(new THREE.BoxGeometry(0.3, 0.3, 1.9), suit); gun.position.set(0.95, -0.4, -1.0); upper.add(gun);
  const tip = new THREE.Mesh(new THREE.BoxGeometry(0.34, 0.34, 0.25), glow); tip.position.set(0.95, -0.4, -2.0); upper.add(tip);
  const shield = new THREE.Mesh(new THREE.SphereGeometry(3.4, 20, 14), new THREE.MeshBasicMaterial({ color: 0x66e6ff, transparent: true, opacity: .16, blending: THREE.AdditiveBlending, depthWrite: false })); shield.position.y = 3; shield.visible = false; g.add(shield);
  const aura = new THREE.Mesh(new THREE.RingGeometry(1.8, 2.3, 28), new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: .8, side: THREE.DoubleSide, blending: THREE.AdditiveBlending, depthWrite: false })); aura.rotation.x = -Math.PI / 2; aura.position.y = 0.2; aura.visible = false; g.add(aura);
  const tag = label(name, color); tag.position.y = 8.3; g.add(tag);
  g.userData = { upper, tip, shield, aura, tag };
  return g;
}

/* ------------------------------------------------------------------ missiles */
function makeMissile(color) {
  const g = new THREE.Group(), c = new THREE.Color(color);
  const body = new THREE.Mesh(new THREE.CylinderGeometry(0.35, 0.35, 2.4, 10), new THREE.MeshStandardMaterial({ color: 0x3a3560, metalness: .5, roughness: .4 })); body.rotation.x = Math.PI / 2; g.add(body);
  const nose = new THREE.Mesh(new THREE.ConeGeometry(0.35, 0.9, 10), new THREE.MeshBasicMaterial({ color: c.clone().multiplyScalar(2) })); nose.rotation.x = Math.PI / 2; nose.position.z = 1.65; g.add(nose);
  const flame = new THREE.Mesh(new THREE.ConeGeometry(0.32, 2.2, 8), new THREE.MeshBasicMaterial({ color: new THREE.Color('#ffb347').multiplyScalar(3), transparent: true, opacity: .9, blending: THREE.AdditiveBlending, depthWrite: false })); flame.rotation.x = -Math.PI / 2; flame.position.z = -2.3; g.add(flame);
  const n = 48, pos = new Float32Array(n * 3);
  const tg = new THREE.BufferGeometry(); tg.setAttribute('position', new THREE.BufferAttribute(pos, 3));
  const trail = new THREE.Line(tg, new THREE.LineBasicMaterial({ color: new THREE.Color('#ffb347').multiplyScalar(1.6), transparent: true, opacity: .7, blending: THREE.AdditiveBlending, depthWrite: false }));
  trail.frustumCulled = false;
  g.userData = { flame, trail, n, filled: 0 };
  g.scale.setScalar(1.5);
  return g;
}
function trailPush(mis, x, y, z) {
  const u = mis.userData, a = u.trail.geometry.attributes.position.array;
  if (!u.filled) { for (let k = 0; k < u.n; k++) a.set([x, y, z], k * 3); u.filled = 1; }
  a.copyWithin(3, 0, (u.n - 1) * 3); a[0] = x; a[1] = y; a[2] = z; u.trail.geometry.attributes.position.needsUpdate = true;
}
const rmis = new Map();  // owner id -> { mesh, buf }
function makeDrone(color) {
  const g = new THREE.Group(), c = new THREE.Color(color);
  const dark = new THREE.MeshStandardMaterial({ color: 0x2c2848, metalness: .5, roughness: .45 }), glow = new THREE.MeshBasicMaterial({ color: c.clone().multiplyScalar(2) });
  const body = new THREE.Mesh(new THREE.BoxGeometry(1.6, 0.55, 1.6), dark); g.add(body);
  const band = new THREE.Mesh(new THREE.BoxGeometry(1.7, 0.14, 1.7), glow); g.add(band);
  const eye = new THREE.Mesh(new THREE.SphereGeometry(0.28, 10, 8), new THREE.MeshBasicMaterial({ color: new THREE.Color('#ff2a4a').multiplyScalar(2.5) })); eye.position.set(0, -0.15, -0.85); g.add(eye);
  const rotors = [];
  for (const [x, z] of [[1, 1], [1, -1], [-1, 1], [-1, -1]]) {
    const arm = new THREE.Mesh(new THREE.BoxGeometry(0.18, 0.14, 2.2), dark); arm.position.set(x * 0.75, 0.1, z * 0.75); arm.rotation.y = Math.atan2(x, z); g.add(arm);
    const rot = new THREE.Mesh(new THREE.CylinderGeometry(0.95, 0.95, 0.05, 18), new THREE.MeshBasicMaterial({ color: c.clone().multiplyScalar(1.2), transparent: true, opacity: .45, blending: THREE.AdditiveBlending, depthWrite: false }));
    rot.position.set(x * 1.5, 0.3, z * 1.5); g.add(rot); rotors.push(rot);
    const bar = new THREE.Mesh(new THREE.BoxGeometry(1.8, 0.04, 0.16), new THREE.MeshBasicMaterial({ color: 0xffffff })); bar.position.copy(rot.position); bar.position.y += 0.03; g.add(bar); rotors.push(bar);
  }
  g.scale.setScalar(1.3);
  g.userData = { trail: null, rotors, eye, drone: true };
  return g;
}
function dropMissile(id) { const r = rmis.get(id); if (r) { scene.remove(r.mesh); if (r.mesh.userData.trail) scene.remove(r.mesh.userData.trail); rmis.delete(id); } }

/* ------------------------------------------------------------------ first-person blaster */
const viewGun = new THREE.Group();
{
  const dark = new THREE.MeshStandardMaterial({ color: 0x5a5488, emissive: 0x241c44, roughness: .4, metalness: .3, depthTest: false });
  const glowM = new THREE.MeshBasicMaterial({ color: 0x22f0ff, depthTest: false });
  const a = new THREE.Mesh(new THREE.BoxGeometry(0.16, 0.2, 0.9), dark); a.position.set(0, 0, 0); viewGun.add(a);
  const b = new THREE.Mesh(new THREE.BoxGeometry(0.12, 0.28, 0.2), dark); b.position.set(0, -0.2, 0.25); b.rotation.x = 0.3; viewGun.add(b);
  const s = new THREE.Mesh(new THREE.BoxGeometry(0.17, 0.04, 0.7), glowM); s.position.set(0, 0.11, -0.05); viewGun.add(s);
  const t = new THREE.Mesh(new THREE.CylinderGeometry(0.06, 0.08, 0.12, 10), glowM); t.rotation.x = Math.PI / 2; t.position.set(0, 0.02, -0.5); viewGun.add(t);
  viewGun.traverse(o => { o.renderOrder = 999; });
  viewGun.position.set(0.36, -0.34, -0.95); viewGun.scale.setScalar(0.5);
  viewGun.userData.glow = glowM;
  camera.add(viewGun);
}

/* ------------------------------------------------------------------ effects: beams and sparks */
const beamGeo = new THREE.CylinderGeometry(1, 1, 1, 8, 1, true); beamGeo.translate(0, 0.5, 0);
const beams = [];
const UP = new THREE.Vector3(0, 1, 0);
function beam(a, b, color, thick = 1) {
  const A = new THREE.Vector3(...a), B = new THREE.Vector3(...b), d = B.clone().sub(A), len = d.length();
  if (len < 0.1) return;
  const c = new THREE.Color(color);
  for (const [w, k, op] of [[0.06 * thick, 3.0, 1.0], [0.25 * thick, 1.0, 0.3]]) {
    const m = new THREE.Mesh(beamGeo, new THREE.MeshBasicMaterial({ color: c.clone().multiplyScalar(k), transparent: true, opacity: op, blending: THREE.AdditiveBlending, depthWrite: false, fog: false }));
    m.position.copy(A); m.scale.set(w, len, w); m.quaternion.setFromUnitVectors(UP, d.clone().normalize());
    scene.add(m); beams.push({ m, life: 0.28, max: 0.28, op });
  }
}
const sparks = [];
function burst(p, color, n = 24, speed = 30, life = 0.6) {
  const g = new THREE.BufferGeometry(), pos = new Float32Array(n * 3), vel = [];
  for (let i = 0; i < n; i++) { pos.set(p, i * 3); const a = Math.random() * Math.PI * 2, u = Math.random() * 2 - 1, r = Math.sqrt(1 - u * u), s = speed * (0.3 + Math.random() * 0.7); vel.push([Math.cos(a) * r * s, Math.abs(u) * s * 0.8 + 4, Math.sin(a) * r * s]); }
  g.setAttribute('position', new THREE.BufferAttribute(pos, 3));
  const m = new THREE.Points(g, new THREE.PointsMaterial({ color: new THREE.Color(color).multiplyScalar(3), size: 0.7, transparent: true, blending: THREE.AdditiveBlending, depthWrite: false }));
  scene.add(m); sparks.push({ m, vel, life, max: life });
}
const blasts = [], blastGeo = new THREE.SphereGeometry(1, 20, 14);
let shake = 0;
function explosion(p, color, big = true, radius) {
  const r = big ? (radius || rules.blast || 18) : 3;
  for (const [c, k, op] of [['#ffb347', 2.5, 0.9], [color, 1.6, 0.45]]) {
    const m = new THREE.Mesh(blastGeo, new THREE.MeshBasicMaterial({ color: new THREE.Color(c).multiplyScalar(k), transparent: true, opacity: op, blending: THREE.AdditiveBlending, depthWrite: false }));
    m.position.set(...p); m.scale.setScalar(0.5); scene.add(m); blasts.push({ m, life: 0.5, max: 0.5, r: c === color ? r : r * 0.7, op });
  }
  burst(p, '#ffb347', big ? 70 : 12, big ? 55 : 12, big ? 1.1 : 0.5); if (big) burst(p, color, 40, 40, 0.9);
  const d = Math.hypot(p[0] - me.x, p[1] - me.y, p[2] - me.z), pan = Math.sin(Math.atan2(p[0] - me.x, -(p[2] - me.z)) - me.yaw);
  if (big) { sfx.boom(Math.min(0.8, 40 / Math.max(d, 1) * 0.8) + 0.05, pan); shake = Math.max(shake, Math.max(0, 1 - d / 90)); } else sfx.fizzle();
}
function stepFx(dt) {
  for (let i = blasts.length - 1; i >= 0; i--) { const b = blasts[i]; b.life -= dt; const f = 1 - b.life / b.max;
    if (b.life <= 0) { scene.remove(b.m); b.m.material.dispose(); blasts.splice(i, 1); continue; }
    b.m.scale.setScalar(0.5 + b.r * Math.sqrt(f)); b.m.material.opacity = b.op * (1 - f) * (1 - f); }
  for (let i = beams.length - 1; i >= 0; i--) { const b = beams[i]; b.life -= dt; if (b.life <= 0) { scene.remove(b.m); b.m.material.dispose(); beams.splice(i, 1); } else b.m.material.opacity = b.op * (b.life / b.max); }
  for (let i = sparks.length - 1; i >= 0; i--) {
    const s = sparks[i]; s.life -= dt;
    if (s.life <= 0) { scene.remove(s.m); s.m.geometry.dispose(); s.m.material.dispose(); sparks.splice(i, 1); continue; }
    const p = s.m.geometry.attributes.position;
    for (let k = 0; k < s.vel.length; k++) { const v = s.vel[k]; v[1] -= 40 * dt; p.array[k * 3] += v[0] * dt; p.array[k * 3 + 1] += v[1] * dt; p.array[k * 3 + 2] += v[2] * dt; }
    p.needsUpdate = true; s.m.material.opacity = s.life / s.max;
  }
}

/* ------------------------------------------------------------------ sound (synthesised, no files) */
let AC = null, master = null;
function audio() { if (!AC) { try { AC = new (window.AudioContext || window.webkitAudioContext)(); master = AC.createGain(); master.gain.value = 0.5; master.connect(AC.destination); } catch (e) { AC = null; } } if (AC && AC.state === 'suspended') AC.resume(); }
function tone({ f0 = 800, f1 = 200, dur = 0.15, type = 'square', vol = 0.3, pan = 0, delay = 0 }) {
  if (!AC) return;
  const t = AC.currentTime + delay, o = AC.createOscillator(), g = AC.createGain(), p = AC.createStereoPanner ? AC.createStereoPanner() : null;
  o.type = type; o.frequency.setValueAtTime(f0, t); o.frequency.exponentialRampToValueAtTime(Math.max(f1, 20), t + dur);
  g.gain.setValueAtTime(vol, t); g.gain.exponentialRampToValueAtTime(0.001, t + dur);
  o.connect(g); if (p) { p.pan.value = Math.max(-1, Math.min(1, pan)); g.connect(p); p.connect(master); } else g.connect(master);
  o.start(t); o.stop(t + dur + 0.02);
}
const sfx = {
  pew(vol = 0.22, pan = 0, over = false) { tone({ f0: over ? 1800 : 1400, f1: over ? 90 : 180, dur: over ? 0.25 : 0.13, type: 'sawtooth', vol, pan }); },
  hit() { tone({ f0: 1900, f1: 1700, dur: 0.06, type: 'square', vol: 0.18 }); tone({ f0: 2500, f1: 2400, dur: 0.05, type: 'square', vol: 0.12, delay: 0.05 }); },
  hurt() { tone({ f0: 180, f1: 50, dur: 0.25, type: 'triangle', vol: 0.5 }); },
  tagged() { [600, 450, 300, 160].forEach((f, i) => tone({ f0: f, f1: f * 0.8, dur: 0.16, type: 'square', vol: 0.2, delay: i * 0.11 })); },
  scored() { [520, 660, 880, 1170].forEach((f, i) => tone({ f0: f, f1: f * 1.02, dur: 0.12, type: 'square', vol: 0.16, delay: i * 0.07 })); },
  pick() { tone({ f0: 300, f1: 1600, dur: 0.35, type: 'sine', vol: 0.35 }); },
  dry() { tone({ f0: 160, f1: 120, dur: 0.06, type: 'square', vol: 0.12 }); },
  overheat() { tone({ f0: 900, f1: 120, dur: 0.5, type: 'sawtooth', vol: 0.2 }); tone({ f0: 3000, f1: 2000, dur: 0.4, type: 'sine', vol: 0.05, delay: 0.1 }); },
  deflect() { tone({ f0: 3000, f1: 1200, dur: 0.12, type: 'triangle', vol: 0.15 }); },
  jump() { tone({ f0: 220, f1: 440, dur: 0.1, type: 'sine', vol: 0.1 }); },
  launch(vol = 0.35, pan = 0) { noise({ dur: 0.7, vol, f0: 600, f1: 3000, pan }); tone({ f0: 200, f1: 900, dur: 0.5, type: 'sawtooth', vol: vol * 0.25, pan }); },
  boom(vol = 0.7, pan = 0) { noise({ dur: 1.2, vol, f0: 1400, f1: 60, pan }); tone({ f0: 110, f1: 30, dur: 0.8, type: 'sine', vol: vol * 0.8, pan }); },
  fizzle() { noise({ dur: 0.4, vol: 0.2, f0: 3000, f1: 400 }); },
  lock() { tone({ f0: 1600, f1: 1600, dur: 0.05, type: 'square', vol: 0.06 }); },
};
let NOISE = null;
function noise({ dur = 0.6, vol = 0.4, f0 = 800, f1 = 100, pan = 0 }) {
  if (!AC) return;
  if (!NOISE) { NOISE = AC.createBuffer(1, AC.sampleRate * 1.5, AC.sampleRate); const d = NOISE.getChannelData(0); for (let i = 0; i < d.length; i++) d[i] = Math.random() * 2 - 1; }
  const t = AC.currentTime, src = AC.createBufferSource(), f = AC.createBiquadFilter(), g = AC.createGain(), p = AC.createStereoPanner ? AC.createStereoPanner() : null;
  src.buffer = NOISE; f.type = 'lowpass'; f.frequency.setValueAtTime(f0, t); f.frequency.exponentialRampToValueAtTime(Math.max(f1, 20), t + dur);
  g.gain.setValueAtTime(vol, t); g.gain.exponentialRampToValueAtTime(0.001, t + dur);
  src.connect(f); f.connect(g); if (p) { p.pan.value = Math.max(-1, Math.min(1, pan)); g.connect(p); p.connect(master); } else g.connect(master);
  src.start(t); src.stop(t + dur + 0.05);
}

/* ------------------------------------------------------------------ state */
const me = { id: 0, x: SIZE / 2, y: 0, z: SIZE / 2, vx: 0, vz: 0, vy: 0, yaw: 0, pitch: 0, onGround: false, jumps: 0, hp: 3, alive: false, flags: 0, lastFire: -9, deadAt: 0, killer: 0, bob: 0 };
const people = new Map();     // id -> {name,color,bot,score,tagged,best, mesh, buf:[], alive, hp, flags}
let ws = null, rules = { hp: 3, cd: 0.5, rapid: 0.2, range: 650, heat: 0.2, heatRapid: 0.08, cool: 0.2, lock: 2 };
let offset = null;            // server clock - local clock (s)
let roundEnds = 0, interUntil = 0, playing = false, paused = false;
const nowS = () => performance.now() / 1000;
const serverNow = () => nowS() + (offset ?? 0);
let sens = store.get('sens', 1);
const keys = {};
let firing = false, zoomHeld = false, zoomToggle = false, zoom = 0;  // zoom: 0 = normal, 1 = fully zoomed
const FOV = 75, FOV_ZOOM = 25;
const heat = { v: 0, t: 0, lockUntil: 0, warned: false };
function heatNow(t) { if (t < heat.lockUntil) return (heat.lockUntil - t) / rules.lock; return Math.max(0, heat.v - (t - heat.t) * rules.cool); }

function person(id) {
  let p = people.get(id);
  if (!p) { p = { id, name: '?', color: '#ffffff', bot: false, score: 0, tagged: 0, best: 0, mesh: null, buf: [], alive: false, hp: 3, flags: 0 }; people.set(id, p); }
  return p;
}
function ensureMesh(p) {
  if (p.id === me.id) return;
  if (p.mesh && p.mesh.userData.key === p.name + p.color) return;
  if (p.mesh) { scene.remove(p.mesh); }
  p.mesh = makeAvatar(p.color, p.name); p.mesh.userData.key = p.name + p.color; p.mesh.visible = false; scene.add(p.mesh);
}

/* ------------------------------------------------------------------ hud */
const hud = {
  center(text, sub = '', ms = 1800, color) {
    const c = $('center'), s = $('sub'); c.textContent = text; c.style.color = color || ''; s.textContent = sub; c.style.opacity = 1; s.style.opacity = 1;
    clearTimeout(hud._t); if (ms) hud._t = setTimeout(() => { c.style.opacity = 0; s.style.opacity = 0; }, ms);
  },
  feed(parts) {
    const d = document.createElement('div');
    for (const p of parts) { if (typeof p === 'string') d.append(p); else { const b = document.createElement('b'); b.textContent = p.text; b.style.color = p.color; d.append(b); } }
    const f = $('feed'); f.prepend(d); while (f.children.length > 6) f.lastChild.remove();
    setTimeout(() => { d.style.opacity = 0; setTimeout(() => d.remove(), 700); }, 5500);
  },
  hitmark(big) { const h = $('hitmark'); h.classList.toggle('big', !!big); h.classList.add('on'); requestAnimationFrame(() => requestAnimationFrame(() => h.classList.remove('on'))); },
  hurt(fromId) {
    const v = $('vig'); v.classList.add('on'); requestAnimationFrame(() => requestAnimationFrame(() => v.classList.remove('on')));
    const p = people.get(fromId); if (!p || !p.mesh) return;
    const a = Math.atan2(p.mesh.position.x - me.x, -(p.mesh.position.z - me.z)) - me.yaw;
    const i = $('dir').firstElementChild; i.style.transform = `rotate(${a}rad)`; i.classList.add('on'); requestAnimationFrame(() => requestAnimationFrame(() => i.classList.remove('on')));
  },
  hp() { const h = $('hp'); h.replaceChildren(...Array.from({ length: rules.hp }, (_, k) => { const i = document.createElement('i'); if (k >= me.hp) i.className = 'gone'; return i; })); },
  powers() {
    const w = $('powers'), t = serverNow(), out = [];
    for (const [k, v] of Object.entries(POWER)) { const until = me.powerUntil?.[k] || 0; if (until > t) { const s = document.createElement('span'); s.className = 'pw'; s.style.background = v.color; s.textContent = v.name + ' ' + Math.ceil(until - t); out.push(s); } }
    if (me.flags & 1) { const s = document.createElement('span'); s.className = 'pw'; s.style.background = '#66e6ff'; s.textContent = 'Shield'; out.push(s); }
    const flyingDrone = MS.active && MS.kind === 'drone';
    if (me.drones || flyingDrone) { const s = document.createElement('span'); s.className = 'pw'; s.style.background = POWER.drone.color; s.textContent = flyingDrone ? 'Drone flying' : (touchMode ? 'Drone ready' : 'Drone ready · E'); out.push(s); }
    $('tdrone').classList.toggle('hidden', !(me.drones || flyingDrone));
    { const s = document.createElement('span'), left = MS.readyAt - nowS(), flying = MS.active && MS.kind === 'missile'; s.className = 'pw'; s.style.background = flying ? '#ffb347' : left > 0 ? 'rgba(255,255,255,.25)' : '#ffb347';
      s.textContent = flying ? 'Missile flying' : left > 0 ? 'Missile ' + Math.ceil(left) + 's' : (touchMode ? 'Missile ready' : 'Missile ready · Q'); out.push(s); }
    w.replaceChildren(...out);
  },
  ranks() { return [...people.values()].sort((a, b) => b.score - a.score || a.tagged - b.tagged); },
  top() {
    const r = hud.ranks(), i = r.findIndex(p => p.id === me.id), mine = people.get(me.id);
    const ord = n => n + (n % 10 === 1 && n % 100 !== 11 ? 'st' : n % 10 === 2 && n % 100 !== 12 ? 'nd' : n % 10 === 3 && n % 100 !== 13 ? 'rd' : 'th');
    $('rank').textContent = !mine ? '—' : r.length < 2 ? 'Just you so far · share the link' : `${mine.score} tag${mine.score === 1 ? '' : 's'} · ${ord(i + 1)} of ${r.length}`;
    $('me').textContent = mine ? mine.name : '';
  },
  board(tbody, list, cols) {
    tbody.replaceChildren(...list.map(p => { const tr = document.createElement('tr'); if (p.id === me.id) tr.className = 'mine';
      const td = document.createElement('td'); const dot = document.createElement('span'); dot.className = 'dot'; dot.style.color = p.color; td.append(dot, p.name); if (p.bot) { const b = document.createElement('span'); b.className = 'botmark'; b.textContent = 'BOT'; td.append(b); } tr.append(td);
      for (const c of cols) { const t = document.createElement('td'); t.className = 'n'; t.textContent = p[c] ?? 0; tr.append(t); } return tr; }));
  },
};

// minimap: the ground drawn once, people on top
const mini = $('mini'), mctx = mini.getContext('2d'), mbase = document.createElement('canvas');
{
  mbase.width = mbase.height = 360; const x = mbase.getContext('2d'), img = x.createImageData(360, 360);
  for (let py = 0; py < 360; py++) for (let px = 0; px < 360; px++) {
    const wx = px / 360 * SIZE, wz = py / 360 * SIZE, h = ground(wx, wz), e = ground(wx + 4, wz) - ground(wx - 4, wz), n = ground(wx, wz + 4) - ground(wx, wz - 4);
    const sh = Math.max(0, Math.min(1, 0.55 - e * 0.12 + n * 0.12)), t = Math.min(1, h / 140), k = (py * 360 + px) * 4;
    if (wet(wx, wz)) { img.data[k] = 20; img.data[k + 1] = 150 + sh * 60; img.data[k + 2] = 220; }
    else { img.data[k] = (14 + 70 * t) * (0.4 + sh); img.data[k + 1] = (16 + 10 * t) * (0.4 + sh); img.data[k + 2] = (40 + 50 * t) * (0.4 + sh); }
    img.data[k + 3] = 255;
  }
  x.putImageData(img, 0, 0);
}
function drawMini() {
  const s = 360 / SIZE; mctx.drawImage(mbase, 0, 0);
  for (const [x, z, r] of PILLARS) { mctx.fillStyle = '#2a2548'; mctx.beginPath(); mctx.arc(x * s, z * s, Math.max(2, r * s), 0, 7); mctx.fill(); }
  for (const m of spotMeshes) if (m && m.s.up) { mctx.fillStyle = POWER[m.s.kind].color; mctx.save(); mctx.translate(m.s.x * s, m.s.z * s); mctx.rotate(Math.PI / 4); mctx.fillRect(-5, -5, 10, 10); mctx.restore(); }
  // everyone else: a dot in their colour with their name
  mctx.font = 'bold 24px Trebuchet MS, sans-serif'; mctx.textAlign = 'center'; mctx.textBaseline = 'bottom';
  for (const p of people.values()) {
    if (p.id === me.id || !p.alive || !p.mesh) continue;
    const x = p.mesh.position.x * s, y = p.mesh.position.z * s;
    mctx.fillStyle = p.color; mctx.shadowColor = p.color; mctx.shadowBlur = 10; mctx.beginPath(); mctx.arc(x, y, 10, 0, 7); mctx.fill(); mctx.shadowBlur = 0;
    mctx.lineWidth = 3; mctx.strokeStyle = '#fff'; mctx.stroke();
    mctx.lineWidth = 4; mctx.strokeStyle = 'rgba(0,0,0,.75)'; mctx.strokeText(p.name, x, y - 13); mctx.fillStyle = '#fff'; mctx.fillText(p.name, x, y - 13);
  }
  // you: a view cone, a pulsing ring and a big arrow
  if (me.alive) {
    const col = people.get(me.id)?.color || '#fff', x = me.x * s, y = me.z * s, half = camera.fov * Math.PI / 360 * camera.aspect;
    mctx.save(); mctx.translate(x, y); mctx.rotate(me.yaw); mctx.scale(1.5, 1.5);
    const g = mctx.createRadialGradient(0, 0, 0, 0, 0, 70); g.addColorStop(0, 'rgba(255,255,255,.35)'); g.addColorStop(1, 'rgba(255,255,255,0)');
    mctx.fillStyle = g; mctx.beginPath(); mctx.moveTo(0, 0); mctx.arc(0, 0, 70, -Math.PI / 2 - Math.min(half, 1.2), -Math.PI / 2 + Math.min(half, 1.2)); mctx.closePath(); mctx.fill();
    const pulse = (performance.now() / 900) % 1;
    mctx.strokeStyle = col; mctx.globalAlpha = 1 - pulse; mctx.lineWidth = 3; mctx.beginPath(); mctx.arc(0, 0, 10 + pulse * 18, 0, 7); mctx.stroke(); mctx.globalAlpha = 1;
    mctx.fillStyle = col; mctx.strokeStyle = '#fff'; mctx.lineWidth = 3; mctx.shadowColor = col; mctx.shadowBlur = 14;
    mctx.beginPath(); mctx.moveTo(0, -16); mctx.lineTo(11, 12); mctx.lineTo(0, 6); mctx.lineTo(-11, 12); mctx.closePath(); mctx.fill(); mctx.stroke();
    mctx.restore();
    mctx.lineWidth = 4; mctx.strokeStyle = 'rgba(0,0,0,.75)'; mctx.strokeText('YOU', x, y - 28); mctx.fillStyle = '#fff'; mctx.fillText('YOU', x, y - 28);
  }
}

/* ------------------------------------------------------------------ network */
function send(m) { if (ws && ws.readyState === 1) ws.send(JSON.stringify(m)); }
function connect(name, color) {
  return new Promise((resolve, reject) => {
    const url = (location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + location.pathname.replace(/[^/]*$/, '') + 'ws';
    ws = new WebSocket(url);
    let ok = false;
    ws.onopen = () => send({ t: 'join', name, color });
    ws.onmessage = e => { const m = JSON.parse(e.data); if (m.t === 'welcome') { ok = true; resolve(); } else if (m.t === 'full') { reject(new Error('The arena is full (16 players). Try again in a bit.')); } on(m); };
    ws.onerror = () => { if (!ok) reject(new Error('Could not reach the game server.')); };
    ws.onclose = () => { if (ok && playing) disconnected(); };
  });
}
function disconnected() {
  playing = false; document.exitPointerLock?.();
  $('err').textContent = 'Lost connection to the game. Press Play to rejoin.';
  $('hud').classList.add('hidden'); $('touch').classList.add('hidden'); $('pause').classList.add('hidden'); $('lobby').classList.remove('hidden'); $('play').disabled = false;
  for (const p of people.values()) if (p.mesh) scene.remove(p.mesh); people.clear();
}

function on(m) {
  switch (m.t) {
    case 'welcome': {
      me.id = m.id; rules = m.rules; offset = m.ts - nowS();
      buildArena(m.pillars, m.spots); buildRuins(m.boxes || [], m.windows || []);
      roundEnds = nowS() + m.left; interUntil = m.inter ? nowS() + m.inter : 0;
      break;
    }
    case 'roster': {
      const seen = new Set();
      for (const r of m.players) { const p = person(r.id); Object.assign(p, { name: r.name, color: r.color, bot: r.bot, score: r.score, tagged: r.tagged, best: r.best }); ensureMesh(p); seen.add(r.id); }
      for (const [id, p] of people) if (!seen.has(id)) { if (p.mesh) scene.remove(p.mesh); people.delete(id); }
      hud.top(); if (!$('board').classList.contains('hidden')) showBoard(true);
      break;
    }
    case 'left': { const p = people.get(m.id); if (p) { if (p.mesh) scene.remove(p.mesh); people.delete(m.id); } break; }
    case 'snap': {
      const sample = m.ts - nowS();
      if (offset == null || sample > offset) offset = sample; else offset += (sample - offset) * 0.002;
      for (const [id, x, y, z, yaw, pitch, hp, alive, flags] of m.p) {
        const p = person(id); p.hp = hp; p.flags = flags;
        if (id === me.id) {
          if (hp !== me.hp) { me.hp = hp; hud.hp(); }
          me.flags = flags;
          continue;
        }
        p.alive = !!alive;
        p.buf.push({ ts: m.ts, x, y, z, yaw, pitch }); if (p.buf.length > 40) p.buf.shift();
      }
      const live = new Set();
      for (const [id, x, y, z, dx, dy, dz, kd] of m.m || []) {
        if (id === me.id) continue;
        live.add(id);
        let r = rmis.get(id);
        if (r && r.drone !== !!kd) { dropMissile(id); r = null; }
        if (!r) { const p = people.get(id), col = p ? p.color : '#fff'; r = { mesh: kd ? makeDrone(col) : makeMissile(col), buf: [], drone: !!kd }; scene.add(r.mesh); if (r.mesh.userData.trail) scene.add(r.mesh.userData.trail); rmis.set(id, r); }
        r.buf.push({ ts: m.ts, x, y, z, dx, dy, dz }); if (r.buf.length > 30) r.buf.shift();
      }
      for (const id of [...rmis.keys()]) if (!live.has(id)) dropMissile(id);
      break;
    }
    case 'spawn': {
      if (m.id === me.id) {
        [me.x, me.y, me.z] = m.p; me.yaw = m.yaw; me.pitch = 0; me.vx = me.vz = me.vy = 0; me.alive = true; me.hp = rules.hp; me.powerUntil = {}; me.drones = 0; heat.v = 0; heat.lockUntil = 0;
        hud.hp(); viewGun.visible = true;
        if (playing && !interUntil) hud.center('GO!', '', 700, '#7dff3d');
      } else { const p = person(m.id); p.buf.length = 0; p.alive = true; p.buf.push({ ts: serverNow() - 1, x: m.p[0], y: m.p[1], z: m.p[2], yaw: m.yaw, pitch: 0 }); if (p.mesh) { p.mesh.position.set(...m.p); } }
      break;
    }
    case 'beam': {
      const by = people.get(m.by), color = by ? by.color : '#ffffff';
      if (m.by === me.id) {
        if (m.hit) { const v = people.get(m.hit); hud.hitmark(m.over || (v && v.hp <= 1)); sfx.hit(); }
        if (m.mhit) { hud.hitmark(true); sfx.hit(); hud.center('MISSILE DOWN!', '', 1300, '#7dff3d'); }
        break;
      }
      let from = m.o;
      if (by && by.mesh && by.mesh.visible) { const v = new THREE.Vector3(); by.mesh.userData.tip.getWorldPosition(v); from = [v.x, v.y, v.z]; }
      beam(from, m.e, color, m.over ? 2.2 : 1);
      burst(m.e, color, 10, 14, 0.35);
      const dx = from[0] - me.x, dz = from[2] - me.z, d = Math.hypot(dx, dz);
      const pan = Math.sin(Math.atan2(dx, -dz) - me.yaw);
      sfx.pew(Math.min(0.22, 14 / Math.max(d, 1) * 0.22) + 0.02, pan, m.over);
      break;
    }
    case 'hurt': {
      if (m.id === me.id) { me.hp = m.hp; hud.hp(); hud.hurt(m.by); sfx.hurt(); buzz(60); }
      if (m.by === me.id && (m.w === 'm' || m.w === 'd') && m.id !== me.id) { hud.hitmark(false); sfx.hit(); }
      break;
    }
    case 'deflect': { const p = people.get(m.id); if (m.by === me.id) sfx.deflect(); if (p && p.mesh) burst([p.mesh.position.x, p.mesh.position.y + 3, p.mesh.position.z], '#66e6ff', 12, 18, 0.4); break; }
    case 'tag': {
      const v = people.get(m.v), k = people.get(m.k);
      if (v && k) hud.feed(m.v === m.k ? [{ text: v.name, color: v.color }, ' blew themselves up'] : [{ text: k.name, color: k.color }, m.w === 'm' ? ' blasted ' : m.w === 'd' ? ' drone-bombed ' : ' tagged ', { text: v.name, color: v.color }]);
      if (m.v === me.id) {
        if (MS.active) endMissile();
        me.alive = false; me.hp = 0; me.deadAt = nowS(); me.killer = m.k; hud.hp(); viewGun.visible = false; firing = false;
        sfx.tagged(); buzz([80, 60, 160]); burst([me.x, me.y + 3, me.z], people.get(me.id)?.color || '#fff', 40, 30, 0.9);
        hud.center('TAGGED', k ? `by ${k.name} · back in ${Math.round(rules.respawn ?? 3)}s` : '', 2600, '#ff4d6d');
      } else if (v) {
        v.alive = false;
        const pos = v.mesh ? v.mesh.position : null; if (pos) burst([pos.x, pos.y + 3, pos.z], v.color, 40, 30, 0.9);
        if (v.mesh) v.mesh.visible = false;
      }
      if (m.k === me.id && m.v !== me.id) {
        sfx.scored();
        const call = { 2: 'Double tag!', 3: 'On a roll!', 5: 'Unstoppable!', 7: 'Creek legend!', 10: 'Absolutely untaggable!' }[m.streak];
        hud.center(`You tagged ${v ? v.name : 'someone'}!`, call || '', 1600, '#7dff3d');
      } else if (k && m.streak >= 3 && { 3: 1, 5: 1, 7: 1, 10: 1 }[m.streak]) hud.feed([{ text: k.name, color: k.color }, ` is on a ${m.streak}-tag streak`]);
      break;
    }
    case 'pick': {
      setSpot(m.i, false);
      if (m.id === me.id && m.kind === 'drone') { me.drones = 1; sfx.pick(); hud.center('DRONE!', touchMode ? 'Tap DRONE to fly it' : 'Press E to fly it', 1800, POWER.drone.color); }
      else if (m.id === me.id) { me.powerUntil = me.powerUntil || {}; me.powerUntil[m.kind] = serverNow() + m.until; sfx.pick(); hud.center(POWER[m.kind].name.toUpperCase() + '!', { rapid: 'Fire fast', over: 'One hit tags', speed: 'Run fast' }[m.kind], 1200, POWER[m.kind].color); }
      else { const p = people.get(m.id); if (p) hud.feed([{ text: p.name, color: p.color }, ' grabbed ', { text: POWER[m.kind].name, color: POWER[m.kind].color }]); }
      break;
    }
    case 'spot': setSpot(m.i, m.up); break;
    case 'launch': {
      if (m.by === me.id) break;
      const d = Math.hypot(m.p[0] - me.x, m.p[2] - me.z), pan = Math.sin(Math.atan2(m.p[0] - me.x, -(m.p[2] - me.z)) - me.yaw);
      sfx.launch(Math.min(0.35, 20 / Math.max(d, 1) * 0.35) + 0.03, pan);
      const p = people.get(m.by); if (p) hud.feed([{ text: p.name, color: p.color }, m.k === 'drone' ? ' sent out a drone' : ' launched a missile']);
      break;
    }
    case 'boom': {
      dropMissile(m.by);
      if (m.down) {  // shot out of the sky: a harmless bang in mid-air
        const p = people.get(m.by), k = people.get(m.down);
        if (m.by === me.id && MS.active) { [MS.x, MS.y, MS.z] = m.p; endMissile(); }
        explosion(m.p, k ? k.color : '#fff', true, m.k === 'drone' ? rules.dblast : rules.blast);
        if (p && k) hud.feed([{ text: k.name, color: k.color }, ' shot down ', { text: p.name, color: p.color }, m.k === 'drone' ? "'s drone" : "'s missile"]);
        if (m.by === me.id) hud.center(m.k === 'drone' ? 'DRONE SHOT DOWN' : 'MISSILE SHOT DOWN', k ? 'by ' + k.name : '', 1800, '#ff4d6d');
        break;
      }
      if (m.by === me.id) { if (MS.active) endMissile(); if (!m.fizzle) break; }
      const p = people.get(m.by); explosion(m.p, p ? p.color : '#fff', !m.fizzle, m.k === 'drone' ? rules.dblast : rules.blast);
      break;
    }
    case 'feed': hud.feed([m.msg]); break;
    case 'round_end': {
      interUntil = nowS() + m.next; firing = false;
      const list = m.board.map(r => ({ ...r, ...{ id: r.id } })); const w = list.find(r => r.id === m.winner);
      const re = $('roundend'); re.querySelector('.win').textContent = w ? (w.id === me.id ? 'You win the round!' : `${w.name} wins the round!`) : 'Nobody tagged anybody.';
      re.querySelector('.win').style.color = w ? w.color : '';
      hud.board(re.querySelector('tbody'), list, ['score', 'tagged']); re.classList.remove('hidden');
      break;
    }
    case 'round': {
      interUntil = 0; roundEnds = nowS() + m.left; $('roundend').classList.add('hidden');
      for (const s of m.spots) setSpot(s.i, s.up);
      hud.center(`ROUND ${m.no}`, 'Fight!', 1500);
      break;
    }
  }
}

/* ------------------------------------------------------------------ input */
function lock() { if (!touchMode && 'requestPointerLock' in canvas) { try { const r = canvas.requestPointerLock({ unadjustedMovement: true }); if (r && r.catch) r.catch(() => canvas.requestPointerLock()); } catch (e) { canvas.requestPointerLock(); } } }
document.addEventListener('pointerlockchange', () => {
  if (!playing) return;
  if (document.pointerLockElement === canvas) { paused = false; $('pause').classList.add('hidden'); }
  else { paused = true; firing = false; for (const k in keys) keys[k] = false; $('pause').classList.remove('hidden'); }
});
document.addEventListener('mousemove', e => {
  if (!playing || paused || document.pointerLockElement !== canvas) return;
  look(e.movementX * 0.0022, e.movementY * 0.0022);
});
function look(dx, dy) {
  if (MS.active) { MS.yaw += dx * sens * 0.8; MS.pitch = Math.max(-1.45, Math.min(1.45, MS.pitch - dy * sens * 0.8)); return; }
  const zs = camera.fov / FOV;
  me.yaw += dx * sens * zs; me.pitch = Math.max(-1.45, Math.min(1.45, me.pitch - dy * sens * zs));
}
canvas.addEventListener('mousedown', e => { if (!playing) return; if (document.pointerLockElement !== canvas) { lock(); return; } if (MS.active) { if (e.button === 0) detonate(); return; } if (e.button === 0) { audio(); firing = true; heat.warned = false; } if (e.button === 2) zoomHeld = true; });
addEventListener('mouseup', e => { if (e.button === 0) firing = false; if (e.button === 2) zoomHeld = false; });
addEventListener('contextmenu', e => { if (playing) e.preventDefault(); });
addEventListener('keydown', e => {
  if (!playing) return;
  if (e.code === 'Tab') { e.preventDefault(); showBoard(true); return; }
  if (paused) return;
  keys[e.code] = true;
  if (e.code === 'Space') { e.preventDefault(); jump(); }
  if (e.code === 'KeyZ') zoomHeld = true;
  if (e.code === 'KeyQ' && !e.repeat) { audio(); if (MS.active) detonate(); else launchMissile(); }
  if (e.code === 'KeyE' && !e.repeat) { audio(); if (MS.active) detonate(); else launchDrone(); }
});
addEventListener('keyup', e => { keys[e.code] = false; if (e.code === 'KeyZ') zoomHeld = false; if (e.code === 'Tab') { e.preventDefault(); showBoard(false); } });
addEventListener('blur', () => { for (const k in keys) keys[k] = false; firing = false; zoomHeld = false; });
function showBoard(on) {
  const b = $('board'); b.classList.toggle('hidden', !on);
  if (on) hud.board(b.querySelector('tbody'), hud.ranks(), ['score', 'tagged', 'best']);
}
function jump() {
  if (!me.alive || MS.active) return;
  if (me.onGround) { me.vy = JUMP1; me.onGround = false; me.jumps = 1; sfx.jump(); }
  else if (me.jumps < 2) { me.vy = JUMP2; me.jumps = 2; sfx.jump(); burst([me.x, me.y, me.z], '#22f0ff', 10, 10, 0.35); }
}

// touch (phones and tablets): a stick that appears wherever your left thumb lands, drag anywhere on the right to
// look (also while holding FIRE), and buttons. It turns itself on at the first touch, and off again if a mouse is used.
const touch = { mx: 0, mz: 0, stickId: null, lookId: null, fireId: null, lx: 0, ly: 0, fx: 0, fy: 0, cx: 0, cy: 0 };
function setTouchMode(on) {
  touchMode = on; document.body.classList.toggle('touching', on);
  $('touch').classList.toggle('hidden', !(on && playing));
  $('keyhelp').classList.toggle('hidden', on); $('touchhelp').classList.toggle('hidden', !on);
  if (on && document.pointerLockElement) document.exitPointerLock();
  if (on && playing && !$('pause').classList.contains('hidden') && document.pointerLockElement == null) { /* stays paused until Resume */ }
}
const buzz = ms => { try { if (touchMode && navigator.vibrate) navigator.vibrate(ms); } catch (e) { /* not supported */ } };
addEventListener('touchstart', () => { if (!touchMode) setTouchMode(true); }, { capture: true, passive: true });
addEventListener('pointerdown', e => { if (e.pointerType === 'mouse' && touchMode && !matchMedia('(pointer: coarse)').matches) setTouchMode(false); }, { capture: true });
addEventListener('gesturestart', e => e.preventDefault());          // no pinch-zoom in the middle of a game
addEventListener('dblclick', e => { if (touchMode) e.preventDefault(); });
{
  const T = $('touch'), stick = $('stick'), knob = $('knob');
  const radius = () => Math.max(50, Math.min(85, Math.min(innerWidth, innerHeight) * 0.1));
  const lookK = () => 4.2 / Math.max(innerWidth, 320);              // a swipe across the screen turns you about 240°
  const rest = () => { stick.classList.remove('on', 'sprint'); stick.style.left = ''; stick.style.top = ''; knob.style.transform = ''; };
  function press(id, t) {
    buzz(8);
    if (id === 'tfire') {
      if (MS.active) { detonate(); return; }
      audio(); firing = true; heat.warned = false; touch.fireId = t.identifier; touch.fx = t.clientX; touch.fy = t.clientY; $('tfire').classList.add('on');
    } else if (id === 'tmis') { audio(); if (MS.active) detonate(); else launchMissile(); }
    else if (id === 'tdrone') { audio(); if (MS.active) detonate(); else launchDrone(); }
    else if (id === 'tjump') jump();
    else if (id === 'tzoom') { zoomToggle = !zoomToggle; $('tzoom').classList.toggle('on', zoomToggle); }
    else if (id === 'tboard') showBoard($('board').classList.contains('hidden'));
    else if (id === 'tmenu') { paused = true; firing = false; $('pause').classList.remove('hidden'); }
  }
  T.addEventListener('touchstart', e => {
    for (const t of e.changedTouches) {
      const btn = t.target.closest ? t.target.closest('.tbtn') : null;
      if (btn) { press(btn.id, t); continue; }
      if (touch.stickId == null && t.clientX < innerWidth * 0.45) {
        touch.stickId = t.identifier; touch.cx = t.clientX; touch.cy = t.clientY;
        stick.style.left = t.clientX + 'px'; stick.style.top = t.clientY + 'px'; stick.classList.add('on');
      } else if (touch.lookId == null) { touch.lookId = t.identifier; touch.lx = t.clientX; touch.ly = t.clientY; }
    }
    e.preventDefault();
  }, { passive: false });
  T.addEventListener('touchmove', e => {
    for (const t of e.changedTouches) {
      if (t.identifier === touch.stickId) {
        const R = radius(); let dx = t.clientX - touch.cx, dy = t.clientY - touch.cy, d = Math.hypot(dx, dy);
        if (d > R) { touch.cx += dx * (1 - R / d); touch.cy += dy * (1 - R / d); dx *= R / d; dy *= R / d; d = R;   // the stick follows a thumb that slides off it
          stick.style.left = touch.cx + 'px'; stick.style.top = touch.cy + 'px'; }
        knob.style.transform = `translate(calc(-50% + ${dx}px), calc(-50% + ${dy}px))`;
        touch.mx = dx / R; touch.mz = dy / R; stick.classList.toggle('sprint', d > R * 0.92);
      } else if (t.identifier === touch.lookId) {
        const k = lookK(); look((t.clientX - touch.lx) * k, (t.clientY - touch.ly) * k); touch.lx = t.clientX; touch.ly = t.clientY;
      } else if (t.identifier === touch.fireId) {
        const k = lookK(); look((t.clientX - touch.fx) * k, (t.clientY - touch.fy) * k); touch.fx = t.clientX; touch.fy = t.clientY;
      }
    }
    e.preventDefault();
  }, { passive: false });
  const end = e => { for (const t of e.changedTouches) {
    if (t.identifier === touch.stickId) { touch.stickId = null; touch.mx = touch.mz = 0; rest(); }
    if (t.identifier === touch.lookId) touch.lookId = null;
    if (t.identifier === touch.fireId) { touch.fireId = null; firing = false; $('tfire').classList.remove('on'); }
  } };
  T.addEventListener('touchend', end); T.addEventListener('touchcancel', end);
}

/* ------------------------------------------------------------------ menus */
{
  const sw = $('swatches'); let color = store.get('color', PALETTE[Math.floor(Math.random() * PALETTE.length)]);
  if (!PALETTE.includes(color)) color = PALETTE[0];
  for (const c of PALETTE) { const b = document.createElement('button'); b.type = 'button'; b.style.background = c; b.style.color = c; b.setAttribute('aria-label', 'Colour ' + c); b.setAttribute('aria-pressed', c === color); b.onclick = () => { color = c; for (const o of sw.children) o.setAttribute('aria-pressed', o === b); }; sw.append(b); }
  $('name').value = store.get('name', '');
  $('sens').value = sens; $('sens').oninput = e => { sens = +e.target.value; store.set('sens', sens); };
  const glowBtn = $('glow'); glowBtn.textContent = glowOn ? 'On' : 'Off';
  glowBtn.onclick = () => { glowOn = !glowOn; bloom.enabled = glowOn; glowBtn.textContent = glowOn ? 'On' : 'Off'; store.set('glow', glowOn); resize(); };
  const play = async () => {
    const name = $('name').value.trim().slice(0, 16) || 'Player';
    store.set('name', name); store.set('color', color);
    $('err').textContent = ''; $('play').disabled = true; audio();
    try { await connect(name, color); }
    catch (e) { $('err').textContent = e.message; $('play').disabled = false; return; }
    playing = true; paused = false;
    $('lobby').classList.add('hidden'); $('hud').classList.remove('hidden');
    if (touchMode) {
      setTouchMode(true);
      const el = document.documentElement, fs = el.requestFullscreen || el.webkitRequestFullscreen;
      try { const r = fs && fs.call(el); if (r && r.catch) r.catch(() => {}); } catch (e) { /* not allowed: fine */ }
    }
    else lock();
    hud.hp(); hud.top();
  };
  $('play').onclick = play;
  $('name').addEventListener('keydown', e => { if (e.key === 'Enter') play(); });
  $('resume').onclick = () => { if (touchMode) { paused = false; $('pause').classList.add('hidden'); } else lock(); };
  $('leave').onclick = () => { playing = false; try { ws.close(); } catch (e) { /* closed */ } disconnected(); $('err').textContent = ''; };
}

/* ------------------------------------------------------------------ the loop */
let last = nowS(), sendT = 0, miniT = 0, hudT = 0;
const tmp = new THREE.Vector3();
function lerpAngle(a, b, f) { const d = ((b - a + Math.PI * 3) % (Math.PI * 2)) - Math.PI; return a + d * f; }
function myMove(dt) {
  const fwd = [Math.sin(me.yaw), -Math.cos(me.yaw)], right = [Math.cos(me.yaw), Math.sin(me.yaw)];
  let ix = 0, iz = 0;
  if (keys.KeyW || keys.ArrowUp) iz += 1; if (keys.KeyS || keys.ArrowDown) iz -= 1; if (keys.KeyD || keys.ArrowRight) ix += 1; if (keys.KeyA || keys.ArrowLeft) ix -= 1;
  if (touchMode) { ix += touch.mx; iz -= touch.mz; }
  if (MS.active) ix = iz = 0;
  let wx = fwd[0] * iz + right[0] * ix, wz = fwd[1] * iz + right[1] * ix; const wl = Math.hypot(wx, wz); if (wl > 1) { wx /= wl; wz /= wl; }
  const t = serverNow(), inWater = wet(me.x, me.z) && me.onGround;
  const sp = ((keys.ShiftLeft || keys.ShiftRight || (touchMode && Math.hypot(touch.mx, touch.mz) > 0.9)) ? 31 : 21) * ((me.powerUntil?.speed || 0) > t ? 1.5 : 1) * (inWater ? 0.6 : 1);
  const k = Math.min(1, dt * (me.onGround ? 12 : 2.5));
  me.vx += (wx * sp - me.vx) * k; me.vz += (wz * sp - me.vz) * k;
  let nx = me.x + me.vx * dt, nz = me.z + me.vz * dt;
  const step = Math.hypot(nx - me.x, nz - me.z);
  if (me.onGround && ground(nx, nz) - me.y > Math.max(0.5, step * 1.5)) { nx = me.x; nz = me.z; me.vx *= 0.3; me.vz *= 0.3; }
  [nx, nz] = pushOut(nx, nz, me.y);
  me.vy -= GRAV * dt; let ny = me.y + me.vy * dt;
  const fl = floorAt(nx, nz, me.y);
  if (ny <= fl || (me.onGround && me.vy <= 0 && ny - fl < Math.max(1.2, step * 1.6))) { ny = fl; me.vy = 0; me.onGround = true; me.jumps = 0; } else me.onGround = false;
  me.x = nx; me.z = nz; me.y = ny;
  me.bob += me.onGround ? Math.hypot(me.vx, me.vz) * dt * 0.45 : 0;
  if (inWater && Math.hypot(me.vx, me.vz) > 8 && Math.random() < dt * 8) burst([me.x + (Math.random() - .5) * 2, me.y + 0.2, me.z + (Math.random() - .5) * 2], '#22f0ff', 6, 7, 0.4);
}
function tryFire() {
  if (!firing || !me.alive || paused || interUntil || MS.active) return;
  const t = nowS(), rapid = (me.powerUntil?.rapid || 0) > serverNow(), cd = rapid ? rules.rapid : rules.cd;
  if (t < heat.lockUntil) { if (!heat.warned) { heat.warned = true; sfx.dry(); } return; }
  if (t - me.lastFire < cd) return;
  me.lastFire = t;
  heat.v = heatNow(t) + (rapid ? rules.heatRapid : rules.heat); heat.t = t;
  if (heat.v >= 1) { heat.v = 0; heat.lockUntil = t + rules.lock; heat.warned = true; sfx.overheat(); }
  const eye = [me.x, me.y + EYE, me.z], cp = Math.cos(me.pitch), d = [Math.sin(me.yaw) * cp, Math.sin(me.pitch), -Math.cos(me.yaw) * cp];
  let tend = rules.range; const tg = rayGround(eye, d, tend); if (tg != null) tend = tg; const tp = raySolid(eye, d, tend); if (tp != null) tend = tp;
  // the people you see (where they are drawn)
  for (const p of people.values()) {
    if (p.id === me.id || !p.alive || !p.mesh || !p.mesh.visible) continue;
    const c = p.mesh.position, a = [c.x, c.y + 1.4, c.z], b = [c.x, c.y + 5.2, c.z];
    // closest approach ray to vertical segment
    const wx = eye[0] - a[0], wz = eye[2] - a[2], dh = Math.hypot(d[0], d[2]);
    if (dh < 1e-4) continue;
    const s = -(wx * d[0] + wz * d[2]) / (dh * dh); if (s < 0 || s > tend) continue;
    const px = eye[0] + d[0] * s - c.x, pz = eye[2] + d[2] * s - c.z, py = eye[1] + d[1] * s;
    if (Math.hypot(px, pz) < 2.0 && py > a[1] - 2 && py < b[1] + 2) tend = s;
  }
  for (const r of rmis.values()) {  // missiles in the air can be shot down
    const c = r.mesh.position, v = [c.x - eye[0], c.y - eye[1], c.z - eye[2]], s = v[0] * d[0] + v[1] * d[1] + v[2] * d[2];
    if (s > 0 && s < tend && Math.hypot(v[0] - d[0] * s, v[1] - d[1] * s, v[2] - d[2] * s) < 3) tend = s;
  }
  const end = [eye[0] + d[0] * tend, eye[1] + d[1] * tend, eye[2] + d[2] * tend];
  viewGun.updateMatrixWorld(true); const muzzle = new THREE.Vector3(0, 0.02, -0.56).applyMatrix4(viewGun.matrixWorld);
  const over = (me.powerUntil?.over || 0) > serverNow(), my = people.get(me.id);
  beam([muzzle.x, muzzle.y, muzzle.z], end, my ? my.color : '#22f0ff', over ? 2.2 : 1);
  burst(end, my ? my.color : '#22f0ff', 10, 14, 0.35);
  sfx.pew(0.22, 0, over);
  viewGun.userData.kick = 1;
  send({ t: 'fire', o: eye.map(v => +v.toFixed(2)), d: d.map(v => +v.toFixed(4)), rt: +(shownRt ?? serverNow() - INTERP).toFixed(3) });
}
const MS = { active: false, kind: 'missile', x: 0, y: 0, z: 0, vx: 0, vy: 0, vz: 0, yaw: 0, pitch: 0, t0: 0, sendT: 0, readyAt: 0, lock: null, endAt: 0, cam: null };
const dirOf = (yaw, pitch) => [Math.sin(yaw) * Math.cos(pitch), Math.sin(pitch), -Math.cos(yaw) * Math.cos(pitch)];
function launchMissile() {
  const t = nowS();
  if (!playing || !me.alive || MS.active || interUntil || t < MS.readyAt) { if (playing && me.alive && t < MS.readyAt) sfx.dry(); return; }
  const d = dirOf(me.yaw, me.pitch);
  Object.assign(MS, { active: true, kind: 'missile', x: me.x + d[0] * 3, y: me.y + EYE + d[1] * 3, z: me.z + d[2] * 3, yaw: me.yaw, pitch: me.pitch, t0: t, sendT: 0, lock: null, endAt: 0 });
  firing = false; zoomHeld = false; zoomToggle = false; me.vx = me.vz = 0;
  send({ t: 'launch', d: d.map(v => +v.toFixed(4)) });
  sfx.launch();
  $('mlbl').textContent = touchMode ? 'MISSILE · DRAG TO STEER · FIRE: BOOM' : 'MISSILE · STEER · CLICK OR Q TO DETONATE';
  $('mcam').classList.remove('hidden', 'drone');
}
function launchDrone() {
  const t = nowS();
  if (!playing || !me.alive || MS.active || interUntil || !me.drones) { if (playing && me.alive && !me.drones && !MS.active) sfx.dry(); return; }
  const d = dirOf(me.yaw, 0);
  Object.assign(MS, { active: true, kind: 'drone', x: me.x + d[0] * 3, y: me.y + EYE + 2, z: me.z + d[2] * 3, vx: 0, vy: 0, vz: 0, yaw: me.yaw, pitch: me.pitch * 0.5, t0: t, sendT: 0, lock: null, endAt: 0 });
  me.drones = 0; firing = false; zoomHeld = false; zoomToggle = false; me.vx = me.vz = 0;
  send({ t: 'launch', k: 'drone', d: d.map(v => +v.toFixed(4)) });
  tone({ f0: 300, f1: 1200, dur: 0.35, type: 'sine', vol: 0.25 }); tone({ f0: 120, f1: 180, dur: 0.6, type: 'sawtooth', vol: 0.08 });
  $('mlbl').textContent = touchMode ? 'DRONE · STICK TO FLY · FIRE: BOOM' : 'DRONE · WASD + SPACE/SHIFT · CLICK OR E: BOOM';
  $('mcam').classList.remove('hidden'); $('mcam').classList.add('drone'); $('lockbox').classList.add('hidden');
}
function endMissile() { if (MS.kind === 'missile') MS.readyAt = nowS() + (rules.mcd || 12); MS.active = false; MS.endAt = nowS() + 0.8; MS.cam = [MS.x, MS.y, MS.z]; $('mcam').classList.add('hidden'); $('lockbox').classList.add('hidden'); }
function detonate(at) {
  if (!MS.active) return;
  const p = at || [MS.x, MS.y, MS.z];
  send({ t: 'boom', p: p.map(v => +v.toFixed(2)) });
  explosion(p, people.get(me.id)?.color || '#fff', true, MS.kind === 'drone' ? rules.dblast : rules.blast);
  MS.x = p[0]; MS.y = p[1]; MS.z = p[2];
  endMissile();
}
function clearTo(o, c, dist) { const d = [(c[0] - o[0]) / dist, (c[1] - o[1]) / dist, (c[2] - o[2]) / dist]; return rayGround(o, d, dist) == null && raySolid(o, d, dist) == null; }
function stepDrone(dt) {
  const t = nowS(), life = rules.dlife || 20;
  if (t - MS.t0 > life) { detonate(); return; }
  let f = 0, sr = 0, up = 0;
  if (keys.KeyW || keys.ArrowUp) f += 1; if (keys.KeyS || keys.ArrowDown) f -= 1; if (keys.KeyD || keys.ArrowRight) sr += 1; if (keys.KeyA || keys.ArrowLeft) sr -= 1;
  if (keys.Space) up += 1; if (keys.ShiftLeft || keys.ShiftRight || keys.KeyC) up -= 1;
  if (touchMode) { f -= touch.mz; sr += touch.mx; }
  const d = dirOf(MS.yaw, MS.pitch), r = [Math.cos(MS.yaw), 0, Math.sin(MS.yaw)];
  let w = [d[0] * f + r[0] * sr, d[1] * f + up, d[2] * f + r[2] * sr]; const wl = Math.hypot(...w); if (wl > 1) w = w.map(v => v / wl);
  const sp = (rules.dspeed || 40) * 0.95, k = Math.min(1, dt * 4);
  MS.vx += (w[0] * sp - MS.vx) * k; MS.vy += (w[1] * sp - MS.vy) * k; MS.vz += (w[2] * sp - MS.vz) * k;
  const o = [MS.x, MS.y, MS.z], step = [MS.vx * dt, MS.vy * dt, MS.vz * dt], len = Math.hypot(...step);
  if (len > 1e-4) {
    const u = step.map(v => v / len);
    if (raySolid(o, u, len + 1.6) != null || rayGround(o, u, len + 1.6) != null) { MS.vx *= -0.3; MS.vy *= -0.3; MS.vz *= -0.3; }  // bump off walls
    else { MS.x += step[0]; MS.y += step[1]; MS.z += step[2]; }
  }
  MS.x = Math.min(Math.max(MS.x, 3), SIZE - 3); MS.z = Math.min(Math.max(MS.z, 3), SIZE - 3);
  MS.y = Math.min(Math.max(MS.y, ground(MS.x, MS.z) + 1.5), ground(MS.x, MS.z) + 220);
  // fly into someone and it goes off
  for (const p of people.values()) {
    if (p.id === me.id || !p.alive || !p.mesh || !p.mesh.visible) continue;
    if (Math.hypot(p.mesh.position.x - MS.x, p.mesh.position.y + 3 - MS.y, p.mesh.position.z - MS.z) < 3.8) { detonate(); return; }
  }
  if ((MS.sendT -= dt) <= 0) { MS.sendT = 0.05; const h = dirOf(MS.yaw, 0); send({ t: 'ms', p: [+MS.x.toFixed(2), +MS.y.toFixed(2), +MS.z.toFixed(2)], d: h.map(v => +v.toFixed(4)) }); }
  $('mfuel').style.setProperty('--f', Math.max(0, 1 - (t - MS.t0) / life));
  if (Math.random() < dt * 6) tone({ f0: 150 + Math.random() * 30, f1: 140, dur: 0.12, type: 'sawtooth', vol: 0.03 });
}
function stepMissile(dt) {
  const t = nowS(), o = [MS.x, MS.y, MS.z];
  if (t - MS.t0 > (rules.mlife || 8) || MS.y < ground(MS.x, MS.z) || MS.x < 0 || MS.z < 0 || MS.x > SIZE || MS.z > SIZE || MS.y > 700) { detonate(); return; }
  // homing: something in front of it, in the open, pulls it round gently (you can still steer away)
  let d = dirOf(MS.yaw, MS.pitch), best = 0.45, lock = null;
  for (const p of people.values()) {
    if (p.id === me.id || !p.alive || !p.mesh || !p.mesh.visible) continue;
    const c = [p.mesh.position.x, p.mesh.position.y + 3, p.mesh.position.z], v = [c[0] - o[0], c[1] - o[1], c[2] - o[2]], dist = Math.hypot(...v);
    if (dist > 420 || dist < 1) continue;
    const ang = Math.acos(Math.max(-1, Math.min(1, (v[0] * d[0] + v[1] * d[1] + v[2] * d[2]) / dist)));
    if (ang < best && clearTo(o, c, dist)) { best = ang; lock = { p, v, dist }; }
  }
  if (lock && (!MS.lock || MS.lock.p !== lock.p)) sfx.lock();
  MS.lock = lock;
  if (lock) {
    const ty = Math.atan2(lock.v[0], -lock.v[2]), tp = Math.atan2(lock.v[1], Math.hypot(lock.v[0], lock.v[2])), k = 1.3 * dt;
    const dy = ((ty - MS.yaw + Math.PI * 3) % (Math.PI * 2)) - Math.PI;
    MS.yaw += Math.max(-k, Math.min(k, dy)); MS.pitch += Math.max(-k, Math.min(k, tp - MS.pitch));
    d = dirOf(MS.yaw, MS.pitch);
  }
  // fly, and blow up on whatever it meets first
  const step = (rules.mspeed || 95) * dt;
  let hit = null;
  const tg = rayGround(o, d, step + 1); if (tg != null) hit = tg;
  const ts = raySolid(o, d, hit ?? step + 1); if (ts != null) hit = ts;
  for (const p of people.values()) {
    if (p.id === me.id || !p.alive || !p.mesh || !p.mesh.visible) continue;
    const c = [p.mesh.position.x - o[0], p.mesh.position.y + 3 - o[1], p.mesh.position.z - o[2]];
    const s = Math.max(0, Math.min(step + 1, c[0] * d[0] + c[1] * d[1] + c[2] * d[2]));
    if (Math.hypot(c[0] - d[0] * s, c[1] - d[1] * s, c[2] - d[2] * s) < 3.2 && (hit == null || s < hit)) hit = s;
  }
  if (hit != null && hit <= step + 1) { const b = Math.max(0, hit - 0.6); detonate([o[0] + d[0] * b, o[1] + d[1] * b, o[2] + d[2] * b]); return; }
  MS.x += d[0] * step; MS.y += d[1] * step; MS.z += d[2] * step;
  if ((MS.sendT -= dt) <= 0) { MS.sendT = 0.05; send({ t: 'ms', p: [+MS.x.toFixed(2), +MS.y.toFixed(2), +MS.z.toFixed(2)], d: d.map(v => +v.toFixed(4)) }); }
  // the missile camera's dials
  $('mfuel').style.setProperty('--f', Math.max(0, 1 - (t - MS.t0) / (rules.mlife || 8)));
  const lb = $('lockbox');
  if (lock) {
    const v = tmp.set(lock.p.mesh.position.x, lock.p.mesh.position.y + 3, lock.p.mesh.position.z).project(camera);
    if (v.z < 1) { lb.classList.remove('hidden'); lb.style.left = ((v.x + 1) / 2 * innerWidth) + 'px'; lb.style.top = ((1 - v.y) / 2 * innerHeight) + 'px'; lb.dataset.name = lock.p.name + ' · ' + Math.round(lock.dist) + ' ft'; }
  } else lb.classList.add('hidden');
}
function drawMissiles(dt) {
  const rt = serverNow() - INTERP;
  for (const r of rmis.values()) {
    const b = r.buf; if (!b.length) continue;
    let s0 = b[0], s1 = b[b.length - 1];
    for (let i = b.length - 1; i > 0; i--) if (b[i - 1].ts <= rt) { s0 = b[i - 1]; s1 = b[i]; break; }
    const f = s1.ts > s0.ts ? Math.max(0, Math.min(1, (rt - s0.ts) / (s1.ts - s0.ts))) : 1;
    const x = s0.x + (s1.x - s0.x) * f, y = s0.y + (s1.y - s0.y) * f, z = s0.z + (s1.z - s0.z) * f;
    r.mesh.position.set(x, y, z);
    if (r.drone) { r.mesh.rotation.y = -Math.atan2(s1.dx, -s1.dz); const u = r.mesh.userData, tt = nowS(); u.rotors.forEach((o, i) => { o.rotation.y = tt * 40 + i; }); u.eye.visible = (tt * 3 % 1) < 0.6; continue; }
    r.mesh.lookAt(x + s1.dx, y + s1.dy, z + s1.dz);
    r.mesh.userData.flame.scale.setScalar(0.8 + Math.random() * 0.5);
    trailPush(r.mesh, x, y, z);
  }
}
let shownRt = null;  // the server moment the picture on screen shows: shots are judged against it
function drawPeople(dt) {
  const rt = serverNow() - INTERP, t = nowS(); shownRt = rt;
  for (const p of people.values()) {
    if (p.id === me.id || !p.mesh) continue;
    const b = p.buf; if (!b.length) { p.mesh.visible = false; continue; }
    let s0 = b[0], s1 = b[b.length - 1];
    for (let i = b.length - 1; i > 0; i--) if (b[i - 1].ts <= rt) { s0 = b[i - 1]; s1 = b[i]; break; }
    const f = s1.ts > s0.ts ? Math.max(0, Math.min(1, (rt - s0.ts) / (s1.ts - s0.ts))) : 1;
    const m = p.mesh, prev = tmp.copy(m.position);
    m.position.set(s0.x + (s1.x - s0.x) * f, s0.y + (s1.y - s0.y) * f, s0.z + (s1.z - s0.z) * f);
    const moving = prev.distanceTo(m.position) / Math.max(dt, 1e-3);
    m.rotation.y = -lerpAngle(s0.yaw, s1.yaw, f);
    m.userData.upper.rotation.x = s0.pitch + (s1.pitch - s0.pitch) * f;
    m.userData.upper.position.y = 4.4 + (moving > 3 && moving < 60 ? Math.abs(Math.sin(t * 9)) * 0.25 : 0);
    m.visible = p.alive;
    m.userData.shield.visible = !!(p.flags & 1); m.userData.shield.material.opacity = 0.12 + 0.08 * Math.sin(t * 12);
    const pw = p.flags & 2 ? '#ffe23d' : p.flags & 4 ? '#ff4d6d' : p.flags & 8 ? '#7dff3d' : null;
    m.userData.aura.visible = !!pw; if (pw) { m.userData.aura.material.color.set(pw).multiplyScalar(2); m.userData.aura.rotation.z = t * 3; }
  }
}
function frame() {
  requestAnimationFrame(frame);
  const t = nowS(), dt = Math.min(0.05, t - last); last = t;
  terrainU.uTime.value = t;
  {
    const want = playing && me.alive && !paused && !MS.active && (zoomHeld || zoomToggle) ? 1 : 0;
    zoom += (want - zoom) * Math.min(1, dt * 14); if (Math.abs(want - zoom) < 0.002) zoom = want;
    const fov = FOV + (FOV_ZOOM - FOV) * zoom;
    if (Math.abs(camera.fov - fov) > 0.01) { camera.fov = fov; camera.updateProjectionMatrix(); }
    $('scope').style.opacity = Math.max(0, zoom * 1.4 - 0.4); viewGun.visible = me.alive && zoom < 0.6 && !MS.active && !MS.endAt;
    const h = heatNow(t), locked = t < heat.lockUntil, bar = $('heat');
    bar.style.setProperty('--h', h); bar.classList.toggle('locked', locked); bar.classList.toggle('hot', !locked && h > 0.7);
    bar.style.opacity = playing && me.alive && (h > 0.01 || locked) ? 1 : 0;
  }
  if (playing) {
    if (me.alive) {
      if (!paused || touchMode) myMove(dt);
      tryFire();
      const bobY = Math.sin(me.bob * 2) * 0.12;
      camera.position.set(me.x, me.y + EYE + bobY, me.z);
      camera.rotation.set(me.pitch, -me.yaw, 0);
      if (MS.active) { if (!paused || touchMode) (MS.kind === 'drone' ? stepDrone : stepMissile)(dt); if (MS.active) { camera.position.set(MS.x, MS.y, MS.z); camera.rotation.set(MS.pitch, -MS.yaw, 0); viewGun.visible = false; } }
      else if (MS.endAt) {
        if (t < MS.endAt) { const d = dirOf(MS.yaw, MS.pitch); camera.position.set(MS.cam[0] - d[0] * 10, MS.cam[1] - d[1] * 10 + 3, MS.cam[2] - d[2] * 10); camera.lookAt(...MS.cam); viewGun.visible = false; }
        else MS.endAt = 0;
      }
      const kick = viewGun.userData.kick || 0; const gk = Math.min(1, camera.aspect); viewGun.scale.setScalar(0.5 * (0.55 + 0.45 * gk)); viewGun.position.set((0.36 + Math.sin(me.bob) * 0.015) * gk, -0.34 + bobY * 0.2, -0.95 + kick * 0.06); viewGun.userData.kick = Math.max(0, kick - dt * 10);
      const pw = (me.powerUntil?.rapid || 0) > serverNow() ? '#ffe23d' : (me.powerUntil?.over || 0) > serverNow() ? '#ff4d6d' : (people.get(me.id)?.color || '#22f0ff');
      viewGun.userData.glow.color.set(pw).multiplyScalar(1.1);
      if ((sendT -= dt) <= 0) { sendT = 0.05; send({ t: 'st', p: [+me.x.toFixed(2), +me.y.toFixed(2), +me.z.toFixed(2)], yaw: +me.yaw.toFixed(3), pitch: +me.pitch.toFixed(3) }); }
    } else {
      // tagged: float up and look at who did it
      const k = people.get(me.killer), up = Math.min(1, (t - me.deadAt) / 1.2);
      camera.position.set(me.x, me.y + EYE + up * 22, me.z + up * 10);
      if (k && k.mesh && k.mesh.visible) camera.lookAt(k.mesh.position.x, k.mesh.position.y + 3, k.mesh.position.z); else camera.lookAt(me.x, me.y, me.z);
    }
    drawPeople(dt); drawMissiles(dt);
    if (shake > 0) { camera.position.x += (Math.random() - .5) * shake * 1.2; camera.position.y += (Math.random() - .5) * shake * 1.2; shake = Math.max(0, shake - dt * 2.5); }
    if ((hudT -= dt) <= 0) {
      hudT = 0.25;
      const left = interUntil ? interUntil - t : roundEnds - t, s = Math.max(0, Math.ceil(left));
      $('timer').textContent = (interUntil ? 'Next round ' : '') + Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0');
      $('timer').style.color = !interUntil && left < 30 ? '#ff4d6d' : '';
      if (interUntil) $('nextround').textContent = `Next round in ${s}s`;
      hud.powers(); hud.top();
    }
    if ((miniT -= dt) <= 0) { miniT = 0.05; drawMini(); }
  } else {
    // lobby: a slow flight over the arena
    const a = t * 0.05; camera.position.set(SIZE / 2 + Math.sin(a) * 640, 150, SIZE / 2 + Math.cos(a) * 640);
    camera.lookAt(SIZE / 2, 30, SIZE / 2);
    drawPeople(dt);
  }
  for (const m of spotMeshes) if (m) { m.gem.rotation.y = t * 2; m.gem.position.y = 3.2 + Math.sin(t * 2.5 + m.s.i) * 0.5; }
  terrainU.uCam.value.copy(camera.position);
  scene.getObjectByName('sky').position.copy(camera.position); scene.getObjectByName('stars').position.copy(camera.position);
  stepFx(dt);
  composer.render();
}
// for poking at from the console
window.__lt = { me, people, scene, MS, spots: () => spotMeshes.filter(Boolean).map(m => m.s), touch, get touchMode() { return touchMode; }, rmis, ground, launch: launchMissile, detonate: () => detonate(), boxes: () => BOXES, raySolid, fire: v => { firing = v; }, get playing() { return playing; } };
setTouchMode(touchMode);
// a preview of the arena behind the lobby (pillars come from the server once you join)
requestAnimationFrame(frame);
