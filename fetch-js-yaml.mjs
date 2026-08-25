import { execFileSync } from "child_process";
import { mkdirSync, writeFileSync, existsSync } from "fs";
import { join } from "path";
import https from "https";

const ROOT = process.cwd();
const OUT = join(ROOT, "node_modules");
mkdirSync(join(OUT, "js-yaml"), { recursive: true });
const m = JSON.parse((await httpsGet("https://registry.npmjs.org/js-yaml")).toString());
const v = "4.2.0";
const tar = m.versions[v].dist.tarball;
const cache = join(ROOT, "js-yaml.tgz");
if (!existsSync(cache)) {
  const b = await httpsGet(tar);
  writeFileSync(cache, b);
}
execFileSync("python3", [join(ROOT, "extract-tgz.py"), cache, join(OUT, "js-yaml")], { stdio: "pipe" });
console.log("OK js-yaml@" + v);

async function httpsGet(url) {
  return new Promise((res, rej) => {
    const t = setTimeout(() => rej(new Error("to")), 25000);
    https.get(url, (r) => {
      clearTimeout(t);
      const c = []; r.on("data", x => c.push(x));
      r.on("end", () => res(Buffer.concat(c)));
    }).on("error", rej);
  });
}