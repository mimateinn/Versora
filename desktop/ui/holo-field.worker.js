/* Ported from Litora (app/holo-field.worker.js) so Opal is the same material
   across the series. Two differences: the watermark words, and bilinear
   ('low') instead of 'high' smoothing when the coarse field and ink grids are
   upscaled. On a software-rasterised canvas 'high' cost about 90 ms a frame at
   1.5x scale and held the field near 10 fps; bilinear renders the same picture
   (at most 7/255 per channel apart) at about a third of the cost.

   Hologram fluid field. Runs in a worker on a transferred OffscreenCanvas, so
   no per-frame work lands on the UI thread.

   Ground: two-level domain-warped gradient noise on a coarse grid (one sample
   per CELL css px), upscaled with high-quality smoothing. Colour comes from a
   cyclic pastel LUT, so there is no hard seam anywhere in the field.
   Wave: a coarse damped wave field (displacement + velocity, Laplacian
   coupling, a restoring pull, damping). A press kicks it outward at one point
   so a ring travels and settles like water; a fast pointer pushes it aside
   along its path and it springs back. It warps the colour, lights the moving
   ring, and moves and turns the watermark words.
   Watermark: English words on the letter diagonal. Each row rolls one way;
   the next row rolls the other way. Words take their ink from the field.
   Plain mode (every other palette): a flat page colour and the same words in
   one ink colour; no fluid, same roll, brush and ripple. */

const CELL = 14;
const FREQ = 1 / 540;
const ROW = 30;
const SPEED = 8;
const GAP = 30;
const FONT_PX = 8.5;
const WORDS = ['VERSORA', 'TRANSLATE', 'TRADUIRE', 'ÜBERSETZEN', 'TRADUCIR', 'LOCALE'];
const BRUSH_R = 66;
/* Wave constants: speed 380 px/s (stable for dt ≤ 1/60 on a 14 px grid),
   a soft restoring pull (~0.5 Hz) and light damping, so a press sends out a
   visible ring with a trailing wavelet or two that fades in about two seconds. */
const WAVE_C2 = (380 * 380) / (CELL * CELL);
const WAVE_W2 = 9;
const WAVE_G = 2.4;
const BRUSH_ACC = 2600;
const TAP_KICK = 900;
const TAP_R = 34;
const GLOW = 1 / 150;

/* Cyclic stops: cyan, violet, pink, gold, mint. Ground stops are already
   mixed halfway toward the page base, so the field stays a pale material. */
const STOPS = {
  light: {
    ground: [[217, 238, 238], [224, 219, 245], [239, 220, 235], [240, 229, 209], [223, 240, 241]],
    ink: [[58, 138, 150], [112, 92, 190], [176, 84, 140], [166, 120, 50], [56, 146, 116]],
    inkAlpha: 0.36,
    sheen: 20,
    lamp: 12,
  },
  dark: {
    // Night opal: navy with cool teal, indigo and plum; no brown, no amber.
    ground: [[20, 38, 50], [27, 30, 58], [38, 29, 55], [24, 34, 60], [19, 42, 50]],
    ink: [[122, 208, 214], [170, 160, 240], [214, 160, 214], [150, 184, 240], [128, 218, 196]],
    inkAlpha: 0.2,
    sheen: 14,
    lamp: 18,
  },
};

function lut(stops) {
  const out = new Uint8ClampedArray(256 * 3);
  const n = stops.length;
  for (let i = 0; i < 256; i++) {
    const f = (i / 256) * n;
    const k = Math.floor(f);
    const u = f - k;
    const p0 = stops[(k - 1 + n) % n];
    const p1 = stops[k % n];
    const p2 = stops[(k + 1) % n];
    const p3 = stops[(k + 2) % n];
    for (let c = 0; c < 3; c++) {
      // Catmull-Rom through the stops keeps the wheel smooth at every joint.
      const v = 0.5 * ((2 * p1[c]) + (-p0[c] + p2[c]) * u
        + (2 * p0[c] - 5 * p1[c] + 4 * p2[c] - p3[c]) * u * u
        + (-p0[c] + 3 * p1[c] - 3 * p2[c] + p3[c]) * u * u * u);
      out[i * 3 + c] = v;
    }
  }
  return out;
}

const LUT = {
  light: { ground: lut(STOPS.light.ground), ink: lut(STOPS.light.ink) },
  dark: { ground: lut(STOPS.dark.ground), ink: lut(STOPS.dark.ink) },
};

/* 2D gradient noise with a quintic fade. Output is roughly [-1, 1]. */
const PERM = new Uint8Array(512);
{
  const p = new Uint8Array(256);
  for (let i = 0; i < 256; i++) p[i] = i;
  let s = 1234567;
  for (let i = 255; i > 0; i--) {
    s = (Math.imul(s, 1103515245) + 12345) >>> 0;
    const j = s % (i + 1);
    const tmp = p[i]; p[i] = p[j]; p[j] = tmp;
  }
  for (let i = 0; i < 512; i++) PERM[i] = p[i & 255];
}
const GX = [1, -1, 1, -1, 1.4142, -1.4142, 0, 0];
const GY = [1, 1, -1, -1, 0, 0, 1.4142, -1.4142];

function noise(x, y) {
  const xf = Math.floor(x);
  const yf = Math.floor(y);
  const X = xf & 255;
  const Y = yf & 255;
  x -= xf;
  y -= yf;
  const u = x * x * x * (x * (x * 6 - 15) + 10);
  const v = y * y * y * (y * (y * 6 - 15) + 10);
  const a = PERM[X] + Y;
  const b = PERM[X + 1] + Y;
  const g00 = PERM[a] & 7;
  const g10 = PERM[b] & 7;
  const g01 = PERM[a + 1] & 7;
  const g11 = PERM[b + 1] & 7;
  const n00 = GX[g00] * x + GY[g00] * y;
  const n10 = GX[g10] * (x - 1) + GY[g10] * y;
  const n01 = GX[g01] * x + GY[g01] * (y - 1);
  const n11 = GX[g11] * (x - 1) + GY[g11] * (y - 1);
  const nx0 = n00 + u * (n10 - n00);
  const nx1 = n01 + u * (n11 - n01);
  return (nx0 + v * (nx1 - nx0)) * 1.3;
}

function fbm2(x, y) {
  return noise(x, y) * 0.66 + noise(x * 2.03 + 17.1, y * 2.03 - 9.7) * 0.34;
}

function fbm3(x, y) {
  return noise(x, y) * 0.57 + noise(x * 2.01 + 31.7, y * 2.01 + 4.3) * 0.29
    + noise(x * 4.03 - 12.9, y * 4.03 + 23.1) * 0.14;
}

let view = null;
let vctx = null;
let field = null;
let fctx = null;
let ink = null;
let ictx = null;
let glyphs = null;
let gctx = null;
let groundImg = null;
let inkImg = null;
let sprites = null;

let W = 0;
let H = 0;
let DPR = 1;
let cols = 0;
let rows = 0;
let sx = new Float32Array(0);
let sy = new Float32Array(0);
let velX = new Float32Array(0);
let velY = new Float32Array(0);
let tmpX = new Float32Array(0);
let tmpY = new Float32Array(0);
let stirEnergy = 0;
let holes = new Float32Array(0);
let mode = 'field';
let plainGround = 'rgb(237, 240, 247)';
let plainInk = 'rgba(48, 55, 71, .1)';

let tone = 'light';
let live = false;
let running = false;
let quiet = false;
let page = null;

let flowT = 0;
let clock = 0;
let last = 0;
// -Infinity means "draw on the next frame" regardless of worker clock.
let lastGround = -Infinity;
let raf = 0;

let phase = 0.5;
let phaseTarget = 0.5;
let pointer = null;
let prevPointer = null;
let brushFrom = null;
let vx = 0;
let vy = 0;
let lampX = -1e4;
let lampY = -1e4;
let lamp = 0;
let pointerAt = 0;

function resize(width, height, dpr) {
  W = Math.max(1, Math.round(width));
  H = Math.max(1, Math.round(height));
  DPR = Math.max(1, Math.min(3, dpr || 1));
  view.width = Math.round(W * DPR);
  view.height = Math.round(H * DPR);
  cols = Math.ceil(W / CELL) + 2;
  rows = Math.ceil(H / CELL) + 2;
  field = new OffscreenCanvas(cols, rows);
  fctx = field.getContext('2d');
  ink = new OffscreenCanvas(cols, rows);
  ictx = ink.getContext('2d');
  groundImg = fctx.createImageData(cols, rows);
  inkImg = ictx.createImageData(cols, rows);
  glyphs = new OffscreenCanvas(view.width, view.height);
  gctx = glyphs.getContext('2d');
  const n = cols * rows;
  sx = new Float32Array(n);
  sy = new Float32Array(n);
  velX = new Float32Array(n);
  velY = new Float32Array(n);
  tmpX = new Float32Array(n);
  tmpY = new Float32Array(n);
  stirEnergy = 0;
  sprites = null;
}

/* Words are pre-rendered once, already sheared onto the letter diagonal
   (baseline (1, -0.5), stems (0.5, 1)), so a frame is only drawImage calls. */
function buildSprites() {
  const t0 = performance.now();
  sprites = new Map();
  const probe = new OffscreenCanvas(8, 8).getContext('2d');
  const font = `600 ${FONT_PX}px Bahnschrift, "Segoe UI", Arial, sans-serif`;
  probe.font = font;
  for (const word of WORDS) {
    const spaced = word.split('').join(' ');
    const w = probe.measureText(spaced).width;
    const asc = FONT_PX * 0.8;
    const desc = FONT_PX * 0.25;
    // Sheared corners of the text box (u in [0, w], v in [-asc, desc]).
    const corners = [[0, -asc], [w, -asc], [0, desc], [w, desc]].map(([u, v]) => [u + 0.5 * v, -0.5 * u + v]);
    const minX = Math.min(...corners.map((c) => c[0])) - 1;
    const minY = Math.min(...corners.map((c) => c[1])) - 1;
    const maxX = Math.max(...corners.map((c) => c[0])) + 1;
    const maxY = Math.max(...corners.map((c) => c[1])) + 1;
    const canvas = new OffscreenCanvas(Math.ceil((maxX - minX) * DPR), Math.ceil((maxY - minY) * DPR));
    const ctx = canvas.getContext('2d');
    ctx.setTransform(DPR, -0.5 * DPR, 0.5 * DPR, DPR, -minX * DPR, -minY * DPR);
    ctx.font = font;
    ctx.fillStyle = '#fff';
    ctx.textBaseline = 'alphabetic';
    ctx.fillText(spaced, 0, 0);
    sprites.set(word, { canvas, ox: minX, oy: minY, w, cx: w / 2 - 0.5 * asc * 0.5, cy: -0.25 * w - asc * 0.5 });
  }
  spriteSeq = WORDS.map((word) => sprites.get(word));
  spritePeriod = spriteSeq.reduce((sum, s) => sum + s.w + GAP, 0);
  stats.spriteMs = performance.now() - t0;
}
let spriteSeq = [];
let spritePeriod = 0;

function smooth(e0, e1, x) {
  const t = Math.min(1, Math.max(0, (x - e0) / (e1 - e0)));
  return t * t * (3 - 2 * t);
}

/* Pointer brush: accelerate the wave field aside along the segment the
   pointer just travelled; the wave then carries, overshoots once and settles. */
function stir(dt) {
  // No page showing (reader, home): nothing to part, nothing to compute.
  if (live && page && pointer && brushFrom) {
    const speed = Math.hypot(vx, vy);
    const w = smooth(220, 1500, speed);
    if (w > 0.001) {
      const ux = vx / speed;
      const uy = vy / speed;
      const ax = brushFrom.x;
      const ay = brushFrom.y;
      const bx = pointer.x;
      const by = pointer.y;
      const abx = bx - ax;
      const aby = by - ay;
      const ab2 = abx * abx + aby * aby || 1;
      const reach = BRUSH_R * 2.6;
      const c0 = Math.max(0, Math.floor((Math.min(ax, bx) - reach) / CELL));
      const c1 = Math.min(cols - 1, Math.ceil((Math.max(ax, bx) + reach) / CELL));
      const r0 = Math.max(0, Math.floor((Math.min(ay, by) - reach) / CELL));
      const r1 = Math.min(rows - 1, Math.ceil((Math.max(ay, by) + reach) / CELL));
      const push = BRUSH_ACC * w * dt;
      for (let r = r0; r <= r1; r++) {
        const py = r * CELL;
        for (let c = c0; c <= c1; c++) {
          const px = c * CELL;
          const t = Math.min(1, Math.max(0, ((px - ax) * abx + (py - ay) * aby) / ab2));
          const qx = px - (ax + abx * t);
          const qy = py - (ay + aby * t);
          const fall = Math.exp(-(qx * qx + qy * qy) / (BRUSH_R * BRUSH_R));
          if (fall < 0.01) continue;
          // Which side of the stroke this cell is on decides the parting direction.
          const side = (qx * -uy + qy * ux) >= 0 ? 1 : -1;
          const i = r * cols + c;
          velX[i] += (-uy * side * 0.94 + ux * 0.34) * push * fall;
          velY[i] += (ux * side * 0.94 + uy * 0.34) * push * fall;
        }
      }
      stirEnergy = Math.max(stirEnergy, 1);
    }
  }
  if (stirEnergy > 0) wave(dt);
}

/* A press: an outward velocity kick in a small disc. The ring that follows is
   the wave equation's own, not a drawn circle. */
function tap(x, y) {
  const reach = TAP_R * 3;
  const c0 = Math.max(0, Math.floor((x - reach) / CELL));
  const c1 = Math.min(cols - 1, Math.ceil((x + reach) / CELL));
  const r0 = Math.max(0, Math.floor((y - reach) / CELL));
  const r1 = Math.min(rows - 1, Math.ceil((y + reach) / CELL));
  for (let r = r0; r <= r1; r++) {
    const dy = r * CELL - y;
    for (let c = c0; c <= c1; c++) {
      const dx = c * CELL - x;
      const d = Math.hypot(dx, dy) || 1;
      const kick = TAP_KICK * (d / TAP_R) * Math.exp(-(d * d) / (TAP_R * TAP_R));
      const i = r * cols + c;
      velX[i] += (dx / d) * kick;
      velY[i] += (dy / d) * kick;
    }
  }
  stirEnergy = Math.max(stirEnergy, 1);
}

function wave(dt) {
  let left = Math.min(dt, 0.1);
  let peak = 0;
  while (left > 1e-6) {
    const h = Math.min(left, 1 / 60);
    left -= h;
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const i = r * cols + c;
        const l = c > 0 ? i - 1 : i;
        const rr = c < cols - 1 ? i + 1 : i;
        const u = r > 0 ? i - cols : i;
        const d = r < rows - 1 ? i + cols : i;
        tmpX[i] = WAVE_C2 * (sx[l] + sx[rr] + sx[u] + sx[d] - 4 * sx[i]) - WAVE_W2 * sx[i] - WAVE_G * velX[i];
        tmpY[i] = WAVE_C2 * (sy[l] + sy[rr] + sy[u] + sy[d] - 4 * sy[i]) - WAVE_W2 * sy[i] - WAVE_G * velY[i];
      }
    }
    peak = 0;
    for (let i = 0; i < sx.length; i++) {
      velX[i] += tmpX[i] * h;
      velY[i] += tmpY[i] * h;
      sx[i] += velX[i] * h;
      sy[i] += velY[i] * h;
      const m = Math.abs(sx[i]) + Math.abs(sy[i]) + (Math.abs(velX[i]) + Math.abs(velY[i])) * 0.02;
      if (m > peak) peak = m;
    }
  }
  stirEnergy = peak;
  // Under a twentieth of a pixel is at rest.
  if (peak < 0.05) {
    sx.fill(0);
    sy.fill(0);
    velX.fill(0);
    velY.fill(0);
    stirEnergy = 0;
  }
}

function sampleStir(x, y, out) {
  const fx = Math.min(cols - 1.001, Math.max(0, x / CELL));
  const fy = Math.min(rows - 1.001, Math.max(0, y / CELL));
  const c = Math.floor(fx);
  const r = Math.floor(fy);
  const u = fx - c;
  const v = fy - r;
  const i = r * cols + c;
  const j = i + cols;
  out[0] = (sx[i] * (1 - u) + sx[i + 1] * u) * (1 - v) + (sx[j] * (1 - u) + sx[j + 1] * u) * v;
  out[1] = (sy[i] * (1 - u) + sy[i + 1] * u) * (1 - v) + (sy[j] * (1 - u) + sy[j + 1] * u) * v;
}

function paintGround(withInk) {
  const pal = STOPS[tone];
  const lg = LUT[tone].ground;
  const li = LUT[tone].ink;
  const gd = groundImg.data;
  const id = inkImg.data;
  const t = flowT;
  const inkA = Math.round(pal.inkAlpha * 255);
  const lampR2 = 1 / (240 * 240);
  const hueBase = 0.5 + phase * 0.5;
  const warp = FREQ * 2.4;
  // Time offsets are the same for every cell; hoist them out of the loop.
  const ax = t * 0.021;
  const ay = -t * 0.013;
  const bx = 5.2 - t * 0.017;
  const by = 1.3 + t * 0.019;
  const cx = 1.7 + t * 0.009;
  const cy = 9.2 - t * 0.011;
  let o = 0;
  let i = 0;
  for (let r = 0; r < rows; r++) {
    const y = r * CELL;
    const fy = y * FREQ;
    for (let c = 0; c < cols; c++, i++) {
      const x = c * CELL;
      const px = x * FREQ - sx[i] * warp;
      const py = fy - sy[i] * warp;
      const qx = fbm2(px + ax, py + ay);
      const qy = fbm2(px + bx, py + by);
      const v = fbm3(px + 2.8 * qx + cx, py + 2.8 * qy + cy);
      let h = hueBase + qx * 0.42 + v * 0.34;
      h -= Math.floor(h);
      const gi = (h * 256) | 0;
      let hi = h + 0.5;
      hi -= Math.floor(hi);
      const ii = (hi * 256) | 0;
      const sheen = smooth(0.05, 0.75, v);
      const dx = x - lampX;
      const dy = y - lampY;
      let lit = lamp > 0 ? lamp * Math.exp(-(dx * dx + dy * dy) * lampR2) : 0;
      // Moving water catches the light: the ring glows where the wave travels.
      if (stirEnergy > 0) lit += Math.min(1, (Math.abs(velX[i]) + Math.abs(velY[i])) * GLOW);
      const lift = (sheen - 0.4) * pal.sheen + lit * (pal.lamp * (0.45 + sheen));
      gd[o] = lg[gi * 3] + lift;
      gd[o + 1] = lg[gi * 3 + 1] + lift;
      gd[o + 2] = lg[gi * 3 + 2] + lift;
      gd[o + 3] = 255;
      if (withInk) {
        id[o] = li[ii * 3];
        id[o + 1] = li[ii * 3 + 1];
        id[o + 2] = li[ii * 3 + 2];
        id[o + 3] = inkA;
      }
      o += 4;
    }
  }
  fctx.putImageData(groundImg, 0, 0);
  // No watermark on this route (reader, home): the ink map is not needed.
  if (withInk) ictx.putImageData(inkImg, 0, 0);
}

/* Measured, not guessed: frame costs (EMA, ms) and the last glyph pass. */
const stats = { initAt: 0, firstMs: 0, spriteMs: 0, groundMs: 0, glyphMs: 0, stirMs: 0, frames: 0, words: 0, moved: 0, turned: 0, maxTurn: 0, maxShift: 0 };
const ema = (prev, next) => (prev ? prev * 0.9 + next * 0.1 : next);

const disp = [0, 0];
const TURN_GAIN = 1.6;
const TURN_MAX = 0.7;

/* A brushed word is moved by the stir at both of its ends. The rigid fit of
   those two moved ends gives its new centre and its new angle, so words on
   either side of a stroke lean opposite ways, then settle with the field. */
function brushed(s, gx, gy) {
  const ex = gx - FONT_PX * 0.2;
  const ey = gy - FONT_PX * 0.4;
  sampleStir(ex, ey, disp);
  const ax = disp[0];
  const ay = disp[1];
  sampleStir(ex + s.w, ey - 0.5 * s.w, disp);
  const bx = disp[0];
  const by = disp[1];
  const cx = gx + s.cx + (ax + bx) * 0.5;
  const cy = gy + s.cy + (ay + by) * 0.5;
  const vx0 = s.w;
  const vy0 = -0.5 * s.w;
  const vx1 = vx0 + bx - ax;
  const vy1 = vy0 + by - ay;
  let turn = Math.atan2(vx0 * vy1 - vy0 * vx1, vx0 * vx1 + vy0 * vy1) * TURN_GAIN;
  turn = Math.max(-TURN_MAX, Math.min(TURN_MAX, turn));
  const shift = Math.hypot(ax + bx, ay + by) * 0.5;
  if (shift > 1) stats.moved++;
  if (shift > stats.maxShift) stats.maxShift = shift;
  if (Math.abs(turn) > 0.05) stats.turned++;
  if (Math.abs(turn) > stats.maxTurn) stats.maxTurn = Math.abs(turn);
  if (Math.abs(turn) < 0.003) {
    gctx.drawImage(s.canvas, (cx - s.cx + s.ox) * DPR, (cy - s.cy + s.oy) * DPR);
    return;
  }
  const c = Math.cos(turn) * DPR;
  const n = Math.sin(turn) * DPR;
  gctx.setTransform(c, n, -n, c, cx * DPR, cy * DPR);
  gctx.drawImage(s.canvas, s.ox - s.cx, s.oy - s.cy, s.canvas.width / DPR, s.canvas.height / DPR);
  gctx.setTransform(1, 0, 0, 1, 0, 0);
}

function paintGlyphs(now) {
  gctx.setTransform(1, 0, 0, 1, 0, 0);
  gctx.globalCompositeOperation = 'source-over';
  gctx.clearRect(0, 0, glyphs.width, glyphs.height);
  if (!page || page.w < 8 || page.h < 8) return false;
  if (!sprites) buildSprites();
  stats.words = 0;
  stats.moved = 0;
  stats.turned = 0;
  gctx.save();
  gctx.beginPath();
  gctx.rect(page.x * DPR, page.y * DPR, page.w * DPR, page.h * DPR);
  gctx.clip();
  const x0 = page.x - 40;
  const y0 = page.y - 40;
  const x1 = page.x + page.w + 40;
  const y1 = page.y + page.h + 40;
  // Inverse of the letter shear: u runs along the baseline, v across rows.
  const corners = [[x0, y0], [x1, y0], [x0, y1], [x1, y1]];
  let vMin = Infinity;
  let vMax = -Infinity;
  for (const [x, y] of corners) {
    const v = (0.5 * x + y) / 1.25;
    if (v < vMin) vMin = v;
    if (v > vMax) vMax = v;
  }
  const t = live ? now / 1000 : 0;
  const period = spritePeriod;
  const seqLen = spriteSeq.length;
  const k0 = Math.floor(vMin / ROW);
  const k1 = Math.ceil(vMax / ROW);
  for (let k = k0; k <= k1; k++) {
    const v = k * ROW;
    // Row k spans u between the page edges at this v.
    let uMin = Infinity;
    let uMax = -Infinity;
    for (const x of [x0, x1]) {
      // x = u + 0.5v and y = -0.5u + v; clamp by both x and y extents.
      const ux = x - 0.5 * v;
      if (ux < uMin) uMin = ux;
      if (ux > uMax) uMax = ux;
    }
    const uyA = (v - y0) / 0.5;
    const uyB = (v - y1) / 0.5;
    uMin = Math.max(uMin, Math.min(uyA, uyB));
    uMax = Math.min(uMax, Math.max(uyA, uyB));
    if (uMax <= uMin) continue;
    const dir = (k & 1) ? -1 : 1;
    const seed = Math.imul(k, 2654435761) >>> 0;
    const shift = (seed % 997) / 997 * period + dir * SPEED * t;
    let idx = (seed >>> 10) % seqLen;
    let u = Math.floor((uMin - shift) / period) * period + shift;
    while (u < uMax + 4) {
      const s = spriteSeq[idx];
      if (u + s.w > uMin - 4) {
        const gx = u + 0.5 * v;
        const gy = -0.5 * u + v;
        stats.words++;
        if (stirEnergy > 0.02) brushed(s, gx, gy);
        else gctx.drawImage(s.canvas, (gx + s.ox) * DPR, (gy + s.oy) * DPR);
      }
      u += s.w + GAP;
      idx = idx + 1 === seqLen ? 0 : idx + 1;
    }
  }
  gctx.restore();
  // Glass surfaces show the fluid, never words: cut the words out under them.
  if (holes.length) {
    gctx.globalCompositeOperation = 'destination-out';
    gctx.fillStyle = '#000';
    gctx.beginPath();
    for (let i = 0; i < holes.length; i += 5) {
      gctx.roundRect(holes[i] * DPR, holes[i + 1] * DPR, holes[i + 2] * DPR, holes[i + 3] * DPR, holes[i + 4] * DPR);
    }
    gctx.fill();
  }
  gctx.globalCompositeOperation = 'source-in';
  if (mode === 'plain') {
    gctx.fillStyle = plainInk;
    gctx.fillRect(0, 0, glyphs.width, glyphs.height);
  } else {
    gctx.imageSmoothingEnabled = true;
    gctx.imageSmoothingQuality = 'low';
    gctx.drawImage(ink, 0, 0, cols, rows, 0, 0, cols * CELL * DPR, rows * CELL * DPR);
  }
  gctx.globalCompositeOperation = 'source-over';
  return true;
}

function compose(now, withGlyphs) {
  if (!stats.firstMs) {
    stats.firstMs = performance.now() - stats.initAt;
    self.postMessage({ type: 'first' });
  }
  vctx.setTransform(1, 0, 0, 1, 0, 0);
  if (mode === 'plain') {
    vctx.fillStyle = plainGround;
    vctx.fillRect(0, 0, view.width, view.height);
  } else {
    vctx.imageSmoothingEnabled = true;
    vctx.imageSmoothingQuality = 'low';
    vctx.drawImage(field, 0, 0, cols, rows, 0, 0, cols * CELL * DPR, rows * CELL * DPR);
  }
  if (withGlyphs && paintGlyphs(now)) vctx.drawImage(glyphs, 0, 0);
}

function track(dt) {
  const k = 1 - Math.exp(-dt / 0.35);
  phase += (phaseTarget - phase) * k;
  if (pointer) {
    const kl = 1 - Math.exp(-dt / 0.18);
    if (lampX < -1e3) { lampX = pointer.x; lampY = pointer.y; }
    lampX += (pointer.x - lampX) * kl;
    lampY += (pointer.y - lampY) * kl;
  }
  const target = pointer && live ? 1 : 0;
  lamp += (target - lamp) * (1 - Math.exp(-dt / 0.4));
  if (!target && lamp < 0.002) lamp = 0;
  // The pointer velocity eases to zero when no new sample arrives.
  if (clock - pointerAt > 0.05) {
    const f = Math.exp(-dt / 0.06);
    vx *= f;
    vy *= f;
  }
}

function frame(nowMs) {
  raf = 0;
  if (!running) return;
  const now = nowMs;
  const dt = last ? Math.min(0.1, Math.max(0, (now - last) / 1000)) : 1 / 60;
  last = now;
  clock += dt;
  const gust = 0.62 + 0.38 * Math.sin(clock * 0.047 + Math.sin(clock * 0.013) * 2.1);
  flowT += dt * gust;
  track(dt);
  const s0 = performance.now();
  stir(dt);
  stats.stirMs = ema(stats.stirMs, performance.now() - s0);
  if (pointer) brushFrom = { x: pointer.x, y: pointer.y };
  // One cadence for field and words: 60 fps while something moves by a
  // visible amount (>1 px), 30 fps at rest, 9 fps when only chrome shows it.
  const busy = stirEnergy > 1 || Math.hypot(vx, vy) > 60;
  // Gaps sit just under 2 and 1 vsync at 60 Hz so rAF jitter cannot halve them.
  const gap = busy ? 14 : quiet ? 110 : 30;
  const glyphsOn = !quiet && !!page;
  if (now - lastGround >= gap) {
    let t0 = performance.now();
    if (mode !== 'plain') paintGround(glyphsOn);
    stats.groundMs = ema(stats.groundMs, performance.now() - t0);
    lastGround = now;
    t0 = performance.now();
    compose(now, glyphsOn);
    stats.glyphMs = ema(stats.glyphMs, performance.now() - t0);
    stats.frames++;
  }
  raf = requestAnimationFrame(frame);
}

function still() {
  // Motion off: one settled picture, no roll, no brush, no lamp.
  sx.fill(0);
  sy.fill(0);
  velX.fill(0);
  velY.fill(0);
  stirEnergy = 0;
  lamp = 0;
  phase = 0.5;
  if (mode !== 'plain') paintGround(!!page);
  compose(0, !!page);
}

function sync() {
  const want = running;
  if (!view || !W) return;
  if (!want) {
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
    return;
  }
  // Motion off, or a plain palette with no page showing (reader, home): one
  // picture is all anyone can see.
  if (!live || (mode === 'plain' && !page)) {
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
    still();
    return;
  }
  last = 0;
  // The very first picture is drawn at once; later frames follow rAF.
  if (!raf) {
    if (!stats.frames) frame(performance.now());
    else raf = requestAnimationFrame(frame);
  }
}

self.onmessage = (event) => {
  const m = event.data || {};
  if (m.type === 'init') {
    stats.initAt = performance.now();
    view = m.canvas;
    vctx = view.getContext('2d', { alpha: false });
    resize(m.width, m.height, m.dpr);
    return;
  }
  if (m.type === 'size') {
    if (!view) return;
    resize(m.width, m.height, m.dpr);
    if (running) { lastGround = -Infinity; sync(); }
    return;
  }
  if (m.type === 'state') {
    tone = m.tone === 'dark' ? 'dark' : 'light';
    live = !!m.live;
    running = !!m.running;
    quiet = !!m.quiet;
    page = m.page || null;
    mode = m.mode === 'plain' ? 'plain' : 'field';
    if (m.ground) plainGround = m.ground;
    if (m.ink) plainInk = m.ink;
    if (!live) { pointer = null; brushFrom = null; vx = 0; vy = 0; }
    lastGround = -Infinity;
    sync();
    return;
  }
  if (m.type === 'pointer') {
    if (!live) return;
    const t = m.t / 1000;
    if (prevPointer) {
      const dt = t - prevPointer.t;
      if (dt > 0.004) {
        const k = 1 - Math.exp(-dt / 0.05);
        vx += ((m.x - prevPointer.x) / dt - vx) * k;
        vy += ((m.y - prevPointer.y) / dt - vy) * k;
      }
    }
    prevPointer = { x: m.x, y: m.y, t };
    if (!pointer) brushFrom = { x: m.x, y: m.y };
    pointer = { x: m.x, y: m.y };
    pointerAt = clock;
    if (Number.isFinite(m.phase)) phaseTarget = m.phase;
    if (running && !raf) sync();
    return;
  }
  if (m.type === 'probe') {
    self.postMessage({ type: 'probe', id: m.id, stats: { ...stats, energy: stirEnergy, mode, holes: holes.length / 5, running, live, quiet, cols, rows } });
    stats.maxTurn = 0;
    stats.maxShift = 0;
    return;
  }
  if (m.type === 'holes') {
    holes = m.rects instanceof Float32Array ? m.rects : new Float32Array(0);
    if (!running) return;
    if (!live) still();
    else lastGround = -Infinity;
    return;
  }
  if (m.type === 'tap') {
    if (!live || !page || !cols || !Number.isFinite(m.x) || !Number.isFinite(m.y)) return;
    tap(m.x, m.y);
    if (running && !raf) sync();
    return;
  }
  if (m.type === 'leave') {
    pointer = null;
    prevPointer = null;
    brushFrom = null;
  }
};
