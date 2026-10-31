// Exercise the built React route in Chrome with the installed ChromeDriver.
// Node's built-in APIs keep this test independent of a second browser library.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function freePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}
async function waitFor(check) {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch { /* The process can still start. */ }
    await delay(100);
  }
  throw new Error("Browser test condition did not become ready");
}

const appPort = await freePort();
const driverPort = await freePort();
const driverUrl = `http://127.0.0.1:${driverPort}`;
const children = [];
let session;
async function command(path, body) {
  const response = await fetch(`${driverUrl}${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: { "content-type": "application/json" },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const result = await response.json();
  if (!response.ok || result.value?.error) throw new Error(JSON.stringify(result.value));
  return result.value;
}
const execute = (script) => command(`/session/${session}/execute/sync`, { script, args: [] });
const painted = () => command(`/session/${session}/execute/async`, {
  script: "const done = arguments[0]; requestAnimationFrame(() => requestAnimationFrame(() => done(document.querySelector('canvas').getAttribute('aria-label'))));",
  args: [],
});
const keyScript = `
  const canvas = document.querySelector('canvas');
  const key = (name, modifiers = {}) => {
    const event = new KeyboardEvent('keydown', { key: name, bubbles: true, cancelable: true, ...modifiers });
    canvas.dispatchEvent(event);
    return event.defaultPrevented;
  };
`;
try {
  children.push(spawn("node", ["node_modules/vite/bin/vite.js", "preview", "--host", "127.0.0.1", "--port", String(appPort), "--strictPort"], { cwd: root, stdio: "ignore" }));
  children.push(spawn("chromedriver", [`--port=${driverPort}`, "--allowed-ips=127.0.0.1"], { stdio: "ignore" }));
  await waitFor(async () => (await fetch(`http://127.0.0.1:${appPort}`)).ok);
  await waitFor(async () => (await command("/status")).ready);
  session = (await command("/session", { capabilities: { alwaysMatch: {
    browserName: "chrome", "goog:chromeOptions": { args: ["--headless=new", "--no-sandbox", "--disable-dev-shm-usage"] },
  } } })).sessionId;
  await command(`/session/${session}/url`, { url: `http://127.0.0.1:${appPort}` });
  await waitFor(() => execute("return !document.querySelector('.add-sheet').disabled"));
  await execute("document.querySelector('canvas').focus()");
  const initial = await painted();
  assert.match(initial, /, A1$/);

  assert.deepEqual(await execute(`${keyScript} return [key('ArrowRight'), key('ArrowRight')];`), [true, true]);
  assert.match(await painted(), /, C1$/);
  assert.deepEqual(await execute(`${keyScript} return [key('ArrowDown'), key('Enter', { shiftKey: true })];`), [true, true]);
  assert.match(await painted(), /, C1$/);

  for (const modifiers of [{ altKey: true }, { isComposing: true }]) {
    assert.equal(await execute(`${keyScript} return key('ArrowDown', ${JSON.stringify(modifiers)});`), false);
    assert.match(await painted(), /, C1$/);
  }
  assert.equal(await execute(`${keyScript}
    const primary = /Mac|iPhone|iPad|iPod/.test(navigator.platform) ? { metaKey: true } : { ctrlKey: true };
    return key('ArrowRight', { ...primary, shiftKey: true });`), false);

  assert.equal(await execute(`${keyScript}
    canvas.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
    const consumed = key('ArrowDown');
    canvas.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true }));
    return consumed;`), false);
  assert.match(await painted(), /, C1$/);

  assert.equal(await execute(`${keyScript}
    const input = document.createElement('input'); document.body.append(input); input.focus();
    const consumed = key('ArrowDown'); input.remove(); canvas.focus(); return consumed;`), false);
  assert.match(await painted(), /, C1$/);

  assert.equal(await execute(`${keyScript} const consumed = key('ArrowRight'); canvas.blur(); return consumed;`), true);
  assert.match(await painted(), /, C1$/);
  assert.equal(await execute(`${keyScript} canvas.focus(); const consumed = key('ArrowRight'); window.dispatchEvent(new Event('blur')); return consumed;`), true);
  assert.match(await painted(), /, C1$/);
  const canvasState = async () => command(`/session/${session}/execute/async`, {
    script: "const done = arguments[0]; requestAnimationFrame(() => requestAnimationFrame(() => done(document.querySelector('canvas').toDataURL())));",
    args: [],
  });
  const beforeWheel = await canvasState();
  await execute(`${keyScript} canvas.dispatchEvent(new WheelEvent('wheel', { deltaY: 30, deltaMode: 0, bubbles: true, cancelable: true })); return true;`);
  await delay(100);
  assert.notEqual(await canvasState(), beforeWheel, "wheel input must repaint the grid");
  console.log("React navigation route: FIFO, reverse Enter, modifiers, IME, input focus, blur, and wheel pass.");
} finally {
  if (session) await fetch(`${driverUrl}/session/${session}`, { method: "DELETE" }).catch(() => {});
  for (const child of children) child.kill();
}
