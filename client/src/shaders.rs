//! GLSL ES 3.00 shaders. Colours are linear and may go above 1 (that is what glows); the output pass tone-maps them
//! (ACES filmic, as three.js does) and writes sRGB.

pub const HEAD: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\n";

/// Lit surfaces, like three.js's MeshStandardMaterial under a hemisphere light and a directional "moon".
pub const STD_VS: &str = r#"
in vec3 aPos; in vec3 aNormal;
uniform mat4 uModel; uniform mat4 uViewProj; uniform mat4 uView;
out vec3 vN; out vec3 vW; out float vDepth;
void main(){
  vec4 w = uModel * vec4(aPos, 1.0);
  vW = w.xyz;
  vN = normalize(transpose(inverse(mat3(uModel))) * aNormal);
  vDepth = -(uView * w).z;
  gl_Position = uViewProj * w;
}"#;

pub const STD_FS: &str = r#"
in vec3 vN; in vec3 vW; in float vDepth;
uniform vec3 uColor; uniform vec3 uEmissive; uniform float uMetal; uniform float uRough;
uniform vec3 uCam; uniform vec3 uSky; uniform vec3 uGround; uniform vec3 uMoon; uniform vec3 uMoonDir;
uniform vec3 uFogColor; uniform float uFogDensity;
out vec4 oColor;
const float PI = 3.141592653589793;
void main(){
  vec3 n = normalize(vN);
  if (!gl_FrontFacing) n = -n;
  vec3 v = normalize(uCam - vW);
  vec3 diffuse = uColor * (1.0 - uMetal);
  vec3 f0 = mix(vec3(0.04), uColor, uMetal);
  vec3 hemi = mix(uGround, uSky, 0.5 * n.y + 0.5);
  vec3 col = hemi * diffuse / PI;
  float nl = clamp(dot(n, uMoonDir), 0.0, 1.0);
  vec3 irr = uMoon * nl;
  col += irr * diffuse / PI;
  float rough = max(uRough, 0.0525);
  float a = rough * rough; float a2 = a * a;
  vec3 h = normalize(uMoonDir + v);
  float nv = clamp(dot(n, v), 0.0, 1.0); float nh = clamp(dot(n, h), 0.0, 1.0); float vh = clamp(dot(v, h), 0.0, 1.0);
  vec3 F = f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
  float gv = nl * sqrt(a2 + (1.0 - a2) * nv * nv); float gl = nv * sqrt(a2 + (1.0 - a2) * nl * nl);
  float V = 0.5 / max(gv + gl, 1e-6);
  float dd = nh * nh * (a2 - 1.0) + 1.0; float D = a2 / (PI * dd * dd);
  col += irr * F * V * D;
  col += uEmissive;
  float fog = 1.0 - exp(-uFogDensity * uFogDensity * vDepth * vDepth);
  oColor = vec4(mix(col, uFogColor, fog), 1.0);
}"#;

/// Flat colour, like MeshBasicMaterial: optional fog and transparency.
pub const BASIC_VS: &str = r#"
in vec3 aPos;
uniform mat4 uModel; uniform mat4 uViewProj; uniform mat4 uView;
out float vDepth;
void main(){ vec4 w = uModel * vec4(aPos, 1.0); vDepth = -(uView * w).z; gl_Position = uViewProj * w; }"#;

pub const BASIC_FS: &str = r#"
in float vDepth;
uniform vec3 uColor; uniform float uOpacity; uniform float uFog; uniform vec3 uFogColor; uniform float uFogDensity;
out vec4 oColor;
void main(){
  float fog = uFog * (1.0 - exp(-uFogDensity * uFogDensity * vDepth * vDepth));
  oColor = vec4(mix(uColor, uFogColor, fog), uOpacity);
}"#;

/// Points with their own colour and alpha: sparks (sized in the world) and stars (sized in pixels).
pub const POINTS_VS: &str = r#"
in vec3 aPos; in vec4 aColor;
uniform mat4 uViewProj; uniform mat4 uView; uniform float uSize; uniform float uScale; uniform float uAtten;
out vec4 vC; out float vDepth;
void main(){
  vec4 mv = uView * vec4(aPos, 1.0);
  vDepth = -mv.z;
  vC = aColor;
  gl_PointSize = uAtten > 0.5 ? uSize * uScale / max(-mv.z, 0.01) : uSize;
  gl_Position = uViewProj * vec4(aPos, 1.0);
}"#;

pub const POINTS_FS: &str = r#"
in vec4 vC; in float vDepth;
uniform float uFog; uniform vec3 uFogColor; uniform float uFogDensity;
out vec4 oColor;
void main(){
  float fog = uFog * (1.0 - exp(-uFogDensity * uFogDensity * vDepth * vDepth));
  oColor = vec4(mix(vC.rgb, uFogColor, fog), vC.a);
}"#;

/// Lines with a colour per vertex (missile trails).
pub const LINE_VS: &str = r#"
in vec3 aPos; in vec4 aColor;
uniform mat4 uViewProj;
out vec4 vC;
void main(){ vC = aColor; gl_Position = uViewProj * vec4(aPos, 1.0); }"#;

pub const LINE_FS: &str = r#"
in vec4 vC; out vec4 oColor; void main(){ oColor = vC; }"#;

/// A camera-facing picture (name tags).
pub const SPRITE_VS: &str = r#"
in vec3 aPos;
uniform mat4 uView; uniform mat4 uProj; uniform vec3 uCenter; uniform vec2 uScale;
out vec2 vUv;
void main(){
  vUv = aPos.xy + 0.5;
  vec4 mv = uView * vec4(uCenter, 1.0);
  mv.xy += aPos.xy * uScale;
  gl_Position = uProj * mv;
}"#;

pub const SPRITE_FS: &str = r#"
in vec2 vUv; uniform sampler2D uTex; out vec4 oColor;
void main(){ vec4 t = texture(uTex, vec2(vUv.x, 1.0 - vUv.y)); oColor = t; }"#;

/// The sky: a gradient dome with a pink glow at the horizon.
pub const SKY_VS: &str = r#"
in vec3 aPos;
uniform mat4 uViewProj; uniform vec3 uCam;
out vec3 vD;
void main(){ vD = normalize(aPos); gl_Position = uViewProj * vec4(aPos + uCam, 1.0); }"#;

pub const SKY_FS: &str = r#"
in vec3 vD; out vec4 oColor;
void main(){
  float h = clamp(vD.y, -0.2, 1.0);
  vec3 hor = vec3(0.32,0.06,0.38), zen = vec3(0.01,0.01,0.05);
  vec3 c = mix(hor, zen, smoothstep(-0.02, 0.45, h));
  c += vec3(1.0,0.25,0.6) * 0.35 * exp(-abs(h) * 18.0);
  oColor = vec4(c, 1.0);
}"#;

/// The ground: height colours, a neon grid, contour lines, and the creek rippling.
pub const TERRAIN_VS: &str = r#"
in vec3 aPos; in vec3 aNormal; in float aWet;
uniform mat4 uViewProj;
out vec3 vP; out vec3 vN; out float vW;
void main(){ vP = aPos; vN = aNormal; vW = aWet; gl_Position = uViewProj * vec4(aPos, 1.0); }"#;

pub const TERRAIN_FS: &str = r#"
uniform float uTime; uniform vec3 uCam;
in vec3 vP; in vec3 vN; in float vW;
out vec4 oColor;
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
  oColor = vec4(col, 1.0);
}"#;

/// The laser fence round the arena.
pub const FENCE_VS: &str = r#"
in vec3 aPos; in vec2 aUv;
uniform mat4 uViewProj;
out vec2 vU;
void main(){ vU = aUv; gl_Position = uViewProj * vec4(aPos, 1.0); }"#;

pub const FENCE_FS: &str = r#"
uniform float uTime; in vec2 vU; out vec4 oColor;
void main(){
  float a = (1.0 - vU.y); a = a*a*0.55;
  float s = smoothstep(0.85, 1.0, abs(sin((vU.y*6.0 - uTime*0.6)*3.14159))) * 0.6;
  oColor = vec4(vec3(1.0,0.24,0.95)*(a + s*(1.0-vU.y)), 1.0);
}"#;

/// Ruined walls: brick shading with neon edges in the wall's tint.
pub const RUINS_VS: &str = r#"
in vec3 aPos; in vec3 aNormal; in vec2 aFuv; in vec2 aFsz; in vec3 aTint;
uniform mat4 uViewProj;
out vec2 vU; out vec2 vS; out vec3 vT; out vec3 vN; out vec3 vP;
void main(){ vU = aFuv; vS = aFsz; vT = aTint; vN = aNormal; vP = aPos; gl_Position = uViewProj * vec4(aPos, 1.0); }"#;

pub const RUINS_FS: &str = r#"
uniform vec3 uCam;
in vec2 vU; in vec2 vS; in vec3 vT; in vec3 vN; in vec3 vP;
out vec4 oColor;
float h(vec2 p){ return fract(sin(dot(p, vec2(12.9898,78.233)))*43758.5453); }
void main(){
  vec3 n = normalize(vN); float ndl = max(dot(n, normalize(vec3(-0.4,0.8,0.3))), 0.0);
  vec2 b = floor(vU * vec2(0.5, 1.0)); float brick = 0.85 + 0.3*h(b);
  vec3 col = vec3(0.13,0.11,0.19) * brick * (0.45 + 0.9*ndl);
  float d = min(min(vU.x, vS.x - vU.x), min(vU.y, vS.y - vU.y));
  col += vT * (1.0 - smoothstep(0.05, 0.22, d)) * 1.3;
  float dist = length(vP - uCam); col = mix(col, vec3(0.05,0.024,0.125), 1.0 - exp(-dist*0.0011));
  oColor = vec4(col, 1.0);
}"#;

/// Full-screen passes.
pub const QUAD_VS: &str = r#"
in vec3 aPos; out vec2 vUv;
void main(){ vUv = aPos.xy * 0.5 + 0.5; gl_Position = vec4(aPos.xy, 0.0, 1.0); }"#;

/// Bloom step 1: keep what is brighter than the threshold.
pub const BRIGHT_FS: &str = r#"
in vec2 vUv; uniform sampler2D uTex; uniform float uThreshold; out vec4 oColor;
void main(){
  vec4 t = texture(uTex, vUv);
  float v = dot(t.rgb, vec3(0.299, 0.587, 0.114));
  float a = smoothstep(uThreshold, uThreshold + 0.01, v);
  oColor = mix(vec4(0.0), t, a);
}"#;

/// Bloom step 2: a separable Gaussian blur.
pub const BLUR_FS: &str = r#"
in vec2 vUv; uniform sampler2D uTex; uniform vec2 uInvSize; uniform vec2 uDir; uniform int uRadius; uniform float uCoef[11];
out vec4 oColor;
void main(){
  float ws = uCoef[0];
  vec3 sum = texture(uTex, vUv).rgb * ws;
  for (int i = 1; i < 11; i++) {
    if (i >= uRadius) break;
    float w = uCoef[i];
    vec2 off = uDir * uInvSize * float(i);
    sum += (texture(uTex, vUv + off).rgb + texture(uTex, vUv - off).rgb) * w;
    ws += 2.0 * w;
  }
  oColor = vec4(sum / ws, 1.0);
}"#;

/// Output: the scene plus the bloom mips, tone-mapped (ACES filmic) and encoded to sRGB.
pub const OUT_FS: &str = r#"
in vec2 vUv;
uniform sampler2D uScene; uniform sampler2D uB0; uniform sampler2D uB1; uniform sampler2D uB2; uniform sampler2D uB3; uniform sampler2D uB4;
uniform float uBloom; uniform float uBloomStrength; uniform float uBloomAlpha; uniform float uF[5]; uniform float uExposure;
out vec4 oColor;
vec3 RRTAndODTFit(vec3 v){ vec3 a = v * (v + 0.0245786) - 0.000090537; vec3 b = v * (0.983729 * v + 0.4329510) + 0.238081; return a / b; }
vec3 aces(vec3 color){
  const mat3 inM = mat3(vec3(0.59719, 0.07600, 0.02840), vec3(0.35458, 0.90834, 0.13383), vec3(0.04823, 0.01566, 0.83777));
  const mat3 outM = mat3(vec3(1.60475, -0.10208, -0.00327), vec3(-0.53108, 1.10813, -0.07276), vec3(-0.07367, -0.00605, 1.07602));
  color *= uExposure / 0.6;
  color = inM * color; color = RRTAndODTFit(color); color = outM * color;
  return clamp(color, 0.0, 1.0);
}
vec3 srgb(vec3 c){ return mix(pow(c, vec3(0.41666)) * 1.055 - vec3(0.055), c * 12.92, vec3(lessThanEqual(c, vec3(0.0031308)))); }
void main(){
  vec3 c = texture(uScene, vUv).rgb;
  if (uBloom > 0.5) {
    vec3 b = uF[0]*texture(uB0, vUv).rgb + uF[1]*texture(uB1, vUv).rgb + uF[2]*texture(uB2, vUv).rgb
           + uF[3]*texture(uB3, vUv).rgb + uF[4]*texture(uB4, vUv).rgb;
    c += uBloomStrength * b * uBloomAlpha;
  }
  oColor = vec4(srgb(aces(c)), 1.0);
}"#;
