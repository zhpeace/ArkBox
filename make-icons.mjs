import zlib from "node:zlib";
import fs from "node:fs";
import path from "node:path";

// ArkBox 图标生成器：内置矢量描述 + 超采样光栅化，无外部依赖。
// 源坐标系固定 1024x1024，渲染时按比例缩放到目标尺寸。

const BG_TOP = [78, 158, 255];
const BG_BOTTOM = [14, 56, 122];
const BOX = [240, 246, 255];
const LID = [125, 175, 225];
const INK = [10, 48, 110];
const GOLD = [248, 197, 105];

const clamp = (v, a, b) => (v < a ? a : v > b ? b : v);

// Apple 风格 squircle：四段三次贝塞尔
function squirclePolygon(steps = 96) {
  const segs = [
    [[512, 0], [728, 0], [1024, 296], [1024, 512]],
    [[1024, 512], [1024, 728], [728, 1024], [512, 1024]],
    [[512, 1024], [296, 1024], [0, 728], [0, 512]],
    [[0, 512], [0, 296], [296, 0], [512, 0]],
  ];
  const pts = [];
  for (const [p0, p1, p2, p3] of segs) {
    for (let i = 0; i < steps; i++) {
      const t = i / steps;
      const u = 1 - t;
      const a = u * u * u,
        b = 3 * u * u * t,
        c = 3 * u * t * t,
        d = t * t * t;
      pts.push([
        a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
        a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
      ]);
    }
  }
  return pts;
}
const SQUIRCLE = squirclePolygon();

function pointInPolygon(x, y, poly) {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i];
    const [xj, yj] = poly[j];
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) {
      inside = !inside;
    }
  }
  return inside;
}

function inRoundRect(x, y, s) {
  if (x < s[0] || x > s[0] + s[2] || y < s[1] || y > s[1] + s[3]) return false;
  const r = s[4];
  const cx = clamp(x, s[0] + r, s[0] + s[2] - r);
  const cy = clamp(y, s[1] + r, s[1] + s[3] - r);
  const dx = x - cx;
  const dy = y - cy;
  return dx * dx + dy * dy <= r * r;
}

function inCircle(x, y, s) {
  const dx = x - (s[0] + s[2]);
  const dy = y - (s[1] + s[3]);
  return dx * dx + dy * dy <= s[4] * s[4];
}

const R = (x, y, w, h, r, fill) => ({ k: "r", s: [x, y, w, h, r], fill });
const C = (cx, cy, r, fill) => ({ k: "c", s: [cx - r, cy - r, r, r, r], fill });

function shapesDetailed() {
  const out = [
    R(140, 200, 744, 630, 76, BOX),
    R(140, 200, 744, 152, 76, LID),
    R(140, 276, 744, 76, 0, LID),
    R(488, 360, 48, 460, 0, INK),
  ];
  const ys = [480, 560, 640, 720];
  for (const ty of ys) {
    out.push(R(412, ty, 84, 40, 14, INK));
    out.push(R(528, ty + 36, 84, 40, 14, INK));
  }
  out.push(R(472, 332, 80, 130, 40, GOLD));
  out.push(R(424, 228, 176, 110, 28, INK));
  out.push(C(512, 282, 22, BOX));
  return out;
}

function shapesSimple() {
  return [
    R(140, 200, 744, 630, 76, BOX),
    R(140, 200, 744, 152, 76, LID),
    R(140, 276, 744, 76, 0, LID),
    R(472, 360, 80, 460, 0, INK),
    R(416, 228, 192, 116, 30, INK),
  ];
}

function sampleShape(x, y, shapes) {
  if (!pointInPolygon(x, y, SQUIRCLE)) return null;
  const t = clamp(y / 1024, 0, 1);
  let color = [
    BG_TOP[0] + (BG_BOTTOM[0] - BG_TOP[0]) * t,
    BG_TOP[1] + (BG_BOTTOM[1] - BG_TOP[1]) * t,
    BG_TOP[2] + (BG_BOTTOM[2] - BG_TOP[2]) * t,
  ];
  for (const sh of shapes) {
    const hit = sh.k === "r" ? inRoundRect(x, y, sh.s) : inCircle(x, y, sh.s);
    if (hit) color = sh.fill;
  }
  return color;
}

// ---- PNG 编码 ----
const crcTable = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = crcTable[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([len, typeBuf, data, crc]);
}

// 渲染单张图标到 raw buffer (RGBA, size*size*4 bytes)
function renderToRaw(size) {
  const ss = size >= 512 ? 2 : size >= 128 ? 3 : 4;
  const shapes = size >= 64 ? shapesDetailed() : shapesSimple();
  const inv = 1 / (ss * ss);
  const buf = Buffer.alloc(size * size * 4);
  let p = 0;
  for (let py = 0; py < size; py++) {
    const ySub = (py + 0.5 / ss) * 1024 / size;
    for (let sx = 0; sx < ss; sx++) {
      const y = (py + (sx + 0.5) / ss) * 1024 / size;
      for (let px = 0; px < size; px++) {
        // 多子采样累计在每个像素内循环
      }
    }
  }
  // 改写：每个像素 ss*ss 次采样
  for (let py = 0; py < size; py++) {
    for (let px = 0; px < size; px++) {
      let r = 0,
        g = 0,
        b = 0,
        a = 0;
      for (let sy = 0; sy < ss; sy++) {
        const y = ((py + (sy + 0.5) / ss) * 1024) / size;
        for (let sx = 0; sx < ss; sx++) {
          const x = ((px + (sx + 0.5) / ss) * 1024) / size;
          const c = sampleShape(x, y, shapes);
          if (c) {
            r += c[0];
            g += c[1];
            b += c[2];
            a += 255;
          }
        }
      }
      buf[p++] = Math.round(r * inv);
      buf[p++] = Math.round(g * inv);
      buf[p++] = Math.round(b * inv);
      buf[p++] = Math.round(a * inv);
    }
  }
  return buf;
}

function rawToPNG(w, h, raw) {
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  const filtered = Buffer.alloc(h * (1 + w * 4));
  for (let i = 0; i < h; i++) {
    filtered[i * (1 + w * 4)] = 0;
    raw.copy(filtered, i * (1 + w * 4) + 1, i * w * 4, (i + 1) * w * 4);
  }
  const idat = zlib.deflateSync(filtered, { level: 9 });
  return Buffer.concat([
    sig,
    chunk("IHDR", ihdr),
    chunk("IDAT", idat),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function renderPNG(size) {
  return rawToPNG(size, size, renderToRaw(size));
}

// ---- ICO：内嵌多尺寸 PNG ----
function makeICO(entries) {
  const dir = Buffer.alloc(6);
  dir.writeUInt16LE(0, 0);
  dir.writeUInt16LE(1, 2);
  dir.writeUInt16LE(entries.length, 4);
  let offset = 6 + 16 * entries.length;
  const parts = [dir];
  for (const [size, png] of entries) {
    const e = Buffer.alloc(16);
    e[0] = size >= 256 ? 0 : size;
    e[1] = size >= 256 ? 0 : size;
    e.writeUInt16LE(1, 4);
    e.writeUInt16LE(32, 6);
    e.writeUInt32LE(png.length, 8);
    e.writeUInt32LE(offset, 12);
    offset += png.length;
    parts.push(e);
  }
  for (const [, png] of entries) parts.push(png);
  return Buffer.concat(parts);
}

const out = path.resolve("src-tauri/icons");
fs.mkdirSync(out, { recursive: true });

const sizes = [16, 32, 48, 64, 128, 256, 512, 1024];
const pngs = {};
for (const s of sizes) {
  pngs[s] = renderPNG(s);
  console.log(`rendered ${s}x${s} (${pngs[s].length} bytes)`);
}

fs.writeFileSync(path.join(out, "32x32.png"), pngs[32]);
fs.writeFileSync(path.join(out, "128x128.png"), pngs[128]);
fs.writeFileSync(path.join(out, "128x128@2x.png"), pngs[256]);
fs.writeFileSync(path.join(out, "icon.png"), pngs[512]);
fs.writeFileSync(
  path.join(out, "icon.ico"),
  makeICO([
    [16, pngs[16]],
    [32, pngs[32]],
    [48, pngs[48]],
    [64, pngs[64]],
    [128, pngs[128]],
    [256, pngs[256]],
  ])
);

// ---- 预览图：把 16/24/32/48/64/96/128/256/512 并排在浅灰底上 ----
function makePreview() {
  const order = [16, 24, 32, 48, 64, 96, 128, 256, 512];
  const pad = 24;
  // 按 16 像素分组（基线对齐），用顶部对齐
  const W =
    pad + order.reduce((a, s) => a + Math.max(s, 60) + pad, 0);
  const H = pad + 512 + pad;
  const raw = Buffer.alloc(W * H * 4);
  // 背景
  for (let i = 0; i < W * H; i++) {
    raw[i * 4] = 245;
    raw[i * 4 + 1] = 245;
    raw[i * 4 + 2] = 247;
    raw[i * 4 + 3] = 255;
  }
  // 把每个图标的 raw 贴到画布（底对齐：每个图标底在 H - pad）
  let x = pad;
  for (const s of order) {
    const tileW = Math.max(s, 60);
    const iconRaw = renderToRaw(s);
    const ox = x + Math.floor((tileW - s) / 2);
    const oy = H - pad - s;
    for (let yy = 0; yy < s; yy++) {
      for (let xx = 0; xx < s; xx++) {
        const srcIdx = (yy * s + xx) * 4;
        const a = iconRaw[srcIdx + 3];
        if (a === 0) continue;
        const dx = ox + xx;
        const dy = oy + yy;
        if (dx < 0 || dx >= W || dy < 0 || dy >= H) continue;
        const dstIdx = (dy * W + dx) * 4;
        const ia = a / 255;
        raw[dstIdx] = Math.round(iconRaw[srcIdx] * ia + raw[dstIdx] * (1 - ia));
        raw[dstIdx + 1] = Math.round(iconRaw[srcIdx + 1] * ia + raw[dstIdx + 1] * (1 - ia));
        raw[dstIdx + 2] = Math.round(iconRaw[srcIdx + 2] * ia + raw[dstIdx + 2] * (1 - ia));
        raw[dstIdx + 3] = 255;
      }
    }
    x += tileW + pad;
  }
  return rawToPNG(W, H, raw);
}

fs.writeFileSync(path.join(out, "icon-preview.png"), makePreview());
console.log("preview written");

// ---- ICNS：交给 macOS 自带 iconutil ----
const iconset = path.resolve("src-tauri/icons/AppIcon.iconset");
fs.rmSync(iconset, { recursive: true, force: true });
fs.mkdirSync(iconset, { recursive: true });
const map = [
  ["icon_16x16.png", 16],
  ["icon_16x16@2x.png", 32],
  ["icon_32x32.png", 32],
  ["icon_32x32@2x.png", 64],
  ["icon_128x128.png", 128],
  ["icon_128x128@2x.png", 256],
  ["icon_256x256.png", 256],
  ["icon_256x256@2x.png", 512],
  ["icon_512x512.png", 512],
  ["icon_512x512@2x.png", 1024],
];
for (const [name, s] of map) fs.writeFileSync(path.join(iconset, name), pngs[s]);
console.log("iconset written to", iconset);
