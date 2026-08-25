import { execFileSync } from "child_process";
import { mkdirSync, writeFileSync, existsSync } from "fs";
import { join } from "path";
import https from "https";

const ROOT = process.cwd();
const OUT = join(ROOT, "node_modules");
const CACHE = join(ROOT, ".tar-cache-missing");
mkdirSync(CACHE, { recursive: true });

const PKGS = [
  ["@deepseek-ai/schemastery", "3.18.1-rc.4"],
  ["@deepseek-ai/cordis-plugin-group", "latest"],
  ["@deepseek-ai/dsh-bash-env", "latest"],
  ["@deepseek-ai/dsh-fs-policy", "latest"],
  ["@deepseek-ai/dsh-permission", "latest"],
  ["@deepseek-ai/dsh-tool-tasks", "latest"],
  ["@deepseek-ai/dsh-skill-local", "latest"],
  ["@deepseek-ai/dsh-tasks-local", "latest"],
  ["@deepseek-ai/dsh-goal-session", "latest"],
  ["@deepseek-ai/dsh-compact-basic", "latest"],
  ["@deepseek-ai/dsh-subagent-fork", "latest"],
  ["@deepseek-ai/dsh-web-app", "latest"],
];

function http(url) {
  return new Promise((res, rej) => {
    const t = setTimeout(() => rej(new Error("timeout")), 25000);
    https.get(url, (r) => {
      clearTimeout(t);
      if (r.statusCode >= 300) { rej(new Error(`http ${r.statusCode}`)); return; }
      const c = []; r.on("data", x => c.push(x));
      r.on("end", () => res(Buffer.concat(c)));
    }).on("error", rej);
  });
}

async function fetchOne([pkg, version]) {
  const ident = `${pkg}@${version}`;
  const m = JSON.parse((await http(`https://registry.npmjs.org/${encodeURIComponent(pkg)}`)).toString());
  const v = version === "latest" ? (m["dist-tags"]?.latest || version) : version;
  if (!m.versions?.[v]) { console.log(`  SKIP ${ident} (no v${v})`); return; }
  const tar = m.versions[v].dist.tarball;
  const slug = pkg.replace(/[^A-Za-z0-9._-]/g,"") + "@" + v + ".tgz";
  const cache = join(CACHE, slug);
  if (!existsSync(cache)) {
    const b = await http(tar);
    writeFileSync(cache, b);
  }
  const target = join(OUT, pkg);
  mkdirSync(target, { recursive: true });
  const helper = join(ROOT, "extract-tgz.py");
  execFileSync("python3", [helper, cache, target], { stdio: "pipe" });
  console.log(`  OK   ${pkg}@${v}`);
}

(async () => {
  for (const p of PKGS) {
    try { await fetchOne(p); } catch(e) { console.log(`  FAIL ${p[0]}: ${e.message}`); }
  }
  console.log("done");
})();