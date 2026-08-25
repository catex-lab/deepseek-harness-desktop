// fetch-pkgs.mjs — pull @deepseek-ai/* tarballs directly from registry
import { execFileSync } from "child_process";
import { existsSync, mkdirSync, writeFileSync, rmSync, readdirSync, statSync } from "fs";
import { join } from "path";
import https from "https";

const REG = "https://registry.npmjs.org";
const OUT = join(process.cwd(), "node_modules");
const CACHE = join(process.cwd(), ".tar-cache");
const CONC = 6;

const seen = new Set();
const queue = [];

function http(url) {
  return new Promise((res, rej) => {
    const t = setTimeout(() => rej(new Error(`to ${url}`)), 20000);
    https.get(url, { headers: { "user-agent": "dsh/1" } }, (r) => {
      clearTimeout(t);
      if (r.statusCode >= 300) { rej(new Error(`http ${r.statusCode}`)); return; }
      const c = []; r.on("data", x => c.push(x));
      r.on("end", () => res(Buffer.concat(c)));
    }).on("error", rej);
  });
}

function resolveVersion(range, meta) {
  // crude: strip range chars, use exact; fallback to latest
  let v = range.replace(/^[\^~>=<\s]+/,"").split(",").map(s=>s.trim()).find(Boolean);
  if (!v) v = "latest";
  if (meta && meta.versions && meta.versions[v]) return v;
  const tags = meta?.["dist-tags"] || {};
  if (range === "latest") return tags.latest || v;
  if (range.startsWith("^") || range.startsWith("~")) {
    // pick highest in same major
    const base = v.split("-")[0].split(".").map(Number);
    const best = Object.keys(meta.versions || {})
      .map(x => [x, x.split(".")[0] === String(base[0])])
      .filter(([,ok]) => ok)
      .sort((a,b) => (b[0] > a[0] ? 1 : -1))[0];
    return best ? best[0] : v;
  }
  return v;
}

async function meta(pkg) {
  if (seen.has(pkg)) return null;
  seen.add(pkg);
  try { return JSON.parse((await http(`${REG}/${encodeURIComponent(pkg)}`)).toString()); }
  catch { console.error(`  metaX ${pkg}`); return null; }
}

async function one(pkg) {
  const m = await meta(pkg);
  if (!m) return;
  const v = resolveVersion("latest", m);
  const tar = m.versions?.[v]?.dist?.tarball;
  if (!tar) { console.error(`  no-tar ${pkg}@${v}`); return; }
  const slug = pkg.replace(/[^A-Za-z0-9._-]/g,"") + "@" + v + ".tgz";
  const cache = join(CACHE, slug);
  let bytes;
  try {
    bytes = await http(tar);
    writeFileSync(cache, bytes);
  } catch(e) { console.error(`  dlX   ${pkg}@${v}: ${e.message}`); return; }
  const target = join(OUT, pkg);
  try {
    mkdirSync(target, { recursive: true });
    const helper = join(process.cwd(), "extract-tgz.py");
    execFileSync("python3", [helper, cache, target], { stdio: "pipe" });
  } catch(e) { console.error(`  tarX  ${pkg}@${v}: ${e.message}`); return; }
  console.log(`  OK ${pkg}@${v} (${(bytes.length/1024).toFixed(1)}KB)`);
  // recurse
  for (const [dn, dr] of Object.entries(m.versions[v].dependencies || {})) {
    const dv = resolveVersion(dr, m.versions);
    if (!seen.has(dn)) queue.push([dn, dr]);
  }
}

async function worker() {
  while (queue.length) {
    const [p] = queue.shift();
    await one(p);
  }
}

(async () => {
  mkdirSync(OUT, { recursive: true });
  mkdirSync(CACHE, { recursive: true });
  const seed = process.argv[2] || "@deepseek-ai/dsh";
  const seedVer = process.argv[3] || "0.1.0-rc.7";
  queue.push([seed]);
  console.log(`seed=${seed} conc=${CONC}`);
  await Promise.all(Array.from({length: CONC}, () => worker()));
  const pkgs = readdirSync(join(OUT, "@deepseek-ai")).length;
  console.log(`done. @deepseek-ai packages: ${pkgs}`);
  rmSync(CACHE, { recursive: true, force: true });
})();