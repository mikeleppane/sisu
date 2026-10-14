// Saves the archify viewer's "SVG · Light" and "SVG · Dark" exports of
// sisu-pipeline.html as sisu-pipeline-{light,dark}.svg, using headless
// Chrome over the DevTools protocol (no npm packages; needs Node 22+).
//   ARCHIFY_CHROME=<chrome> node export-svgs.mjs [diagram.html] [out-dir]
import { spawn } from "node:child_process";
import { mkdtempSync, renameSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const html = resolve(process.argv[2] ?? "sisu-pipeline.html");
const out = resolve(process.argv[3] ?? ".");
const chromePath = process.env.ARCHIFY_CHROME;
if (!chromePath) throw new Error("set ARCHIFY_CHROME to a Chrome or Chromium binary");

const profile = mkdtempSync(join(tmpdir(), "export-svgs-"));
const chrome = spawn(chromePath, [
  "--headless=new", "--remote-debugging-port=0", "--no-first-run",
  `--user-data-dir=${profile}`, "about:blank",
], { stdio: ["ignore", "ignore", "pipe"] });

try {
  const wsUrl = await new Promise((ok, fail) => {
    let err = "";
    chrome.stderr.on("data", (d) => {
      err += d;
      const m = err.match(/DevTools listening on (ws:\S+)/);
      if (m) ok(m[1]);
    });
    chrome.on("exit", () => fail(new Error(`Chrome exited:\n${err}`)));
  });

  const ws = new WebSocket(wsUrl);
  await new Promise((ok) => ws.addEventListener("open", ok, { once: true }));
  let id = 0;
  const pending = new Map();
  const listeners = new Set();
  ws.addEventListener("message", ({ data }) => {
    const msg = JSON.parse(data);
    if (msg.id && pending.has(msg.id)) {
      const { ok, fail } = pending.get(msg.id);
      pending.delete(msg.id);
      msg.error ? fail(new Error(msg.error.message)) : ok(msg.result);
    } else for (const l of listeners) l(msg);
  });
  const send = (method, params = {}, sessionId) => new Promise((ok, fail) => {
    pending.set(++id, { ok, fail });
    ws.send(JSON.stringify({ id, method, params, sessionId }));
  });
  const event = (method, test = () => true) => new Promise((ok) => {
    const l = (msg) => {
      if (msg.method === method && test(msg.params)) { listeners.delete(l); ok(msg.params); }
    };
    listeners.add(l);
  });

  await send("Browser.setDownloadBehavior", { behavior: "allowAndName", downloadPath: out, eventsEnabled: true });
  const { targetId } = await send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
  await send("Page.enable", {}, sessionId);
  const loaded = event("Page.loadEventFired");
  await send("Page.navigate", { url: pathToFileURL(html).href }, sessionId);
  await loaded;

  for (const theme of ["light", "dark"]) {
    const begun = event("Browser.downloadWillBegin");
    const { result } = await send("Runtime.evaluate", {
      expression: `(() => { const b = document.querySelector('button[data-format="svg-${theme}"]'); if (b) b.click(); return !!b; })()`,
      returnByValue: true,
    }, sessionId);
    if (!result.value) throw new Error(`no svg-${theme} export button in ${html}`);
    const { guid } = await begun;
    const done = await event("Browser.downloadProgress", (p) => p.guid === guid && p.state !== "inProgress");
    if (done.state !== "completed") throw new Error(`svg-${theme} download ${done.state}`);
    renameSync(join(out, guid), join(out, `sisu-pipeline-${theme}.svg`));
  }
  ws.close();
} finally {
  chrome.kill();
  rmSync(profile, { recursive: true, force: true });
}
