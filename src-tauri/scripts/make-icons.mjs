// Regenerate the application icon set from the canonical black/white
// transparent source logos.
//
//   icons/src_light.png  -> black logo  (used on LIGHT backgrounds)
//   icons/src_dark.png   -> white logo  (used on DARK  backgrounds)
//
// Outputs:
//   icon-light.ico  black logo on transparent  (tray/titlebar on light)
//   icon-dark.ico   white logo on transparent  (tray/titlebar on dark)
//   icon.ico        black whale logo on TRANSPARENT  (exe / shortcut /
//                   pinned taskbar). No solid tile — just the logo, matching
//                   the tray/titlebar icons.
//
import sharp from "sharp";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ICONS = join(__dirname, "..", "icons");

const SIZES = [16, 24, 32, 48, 64, 128, 256];

// --- ICO container writer (PNG-encoded images, supported since Vista) ------
function encodeIco(pngs) {
  const head = Buffer.alloc(6);
  head.writeUInt16LE(0, 0); // reserved
  head.writeUInt16LE(1, 2); // type = icon
  head.writeUInt16LE(pngs.length, 4);
  let offset = 6 + pngs.length * 16;
  const entries = [];
  for (const p of pngs) {
    const e = Buffer.alloc(16);
    e.writeUInt8(p.size >= 256 ? 0 : p.size, 0); // width (0 => 256)
    e.writeUInt8(p.size >= 256 ? 0 : p.size, 1); // height
    e.writeUInt8(0, 2); // colors (0 => >256)
    e.writeUInt8(0, 3); // reserved
    e.writeUInt16LE(1, 4); // color planes
    e.writeUInt16LE(32, 6); // bits per pixel
    e.writeUInt32LE(p.data.length, 8); // bytes in resource
    e.writeUInt32LE(offset, 12); // offset
    entries.push(e);
    offset += p.data.length;
  }
  return Buffer.concat([head, ...entries, ...pngs.map((p) => p.data)]);
}

async function logoOnTransparent(size, src) {
  const buf = await sharp(join(ICONS, src))
    .resize(size, size, { fit: "contain", background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .png()
    .toBuffer();
  return { size, data: buf };
}

async function main() {
  const light = [];
  const dark = [];
  const master = [];
  for (const s of SIZES) {
    light.push(await logoOnTransparent(s, "src_light.png"));
    dark.push(await logoOnTransparent(s, "src_dark.png"));
    // exe icon: black whale on transparent (matches the brand / tray style)
    master.push(await logoOnTransparent(s, "src_light.png"));
  }
  writeFileSync(join(ICONS, "icon-light.ico"), encodeIco(light));
  writeFileSync(join(ICONS, "icon-dark.ico"), encodeIco(dark));
  writeFileSync(join(ICONS, "icon.ico"), encodeIco(master));
  console.log("wrote icon.ico (black whale, transparent), icon-light.ico, icon-dark.ico");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
