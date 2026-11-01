// Exercise the built React route in Chrome with the installed ChromeDriver.
// Node's built-in APIs keep this test independent of a second browser library.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { createServer } from "node:net";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
// Loading a workbook frees the old model. A later resize must not call it.
const fixture = readFileSync(
  new URL("../../../crates/iron-canvas-ironcalc/tests/fixtures/cf_tests.xlsx", import.meta.url),
).toString("base64");
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
    try {
      if (await check()) return;
    } catch {
      // The server or the app is still starting.
    }
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
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const result = await response.json();
  if (!response.ok || result.value?.error) throw new Error(JSON.stringify(result.value));
  return result.value;
}
const execute = (script) => command(`/session/${session}/execute/sync`, { script, args: [] });
const executeWithArgs = (script, args) => command(`/session/${session}/execute/sync`, { script, args });
const painted = () => command(`/session/${session}/execute/async`, {
  script: "const done = arguments[0]; requestAnimationFrame(() => requestAnimationFrame(() => done(document.querySelector('canvas').getAttribute('aria-label'))));",
  args: [],
});
const selectedTab = () => execute("return document.querySelector('.sheet-tab.selected')?.textContent ?? '';");
const keyScript = `
  const canvas = document.querySelector('canvas');
  const key = (name, modifiers = {}) => {
    const event = new KeyboardEvent('keydown', { key: name, bubbles: true, cancelable: true, ...modifiers });
    canvas.dispatchEvent(event);
    return event.defaultPrevented;
  };
`;
const primary = "const primary = /Mac|iPhone|iPad|iPod/.test(navigator.platform) ? { metaKey: true } : { ctrlKey: true };";
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

  assert.match(await painted(), /, A1$/);
  assert.equal(await selectedTab(), "Sheet1");

  // Plain arrows, Tab, Enter, and Page keys call the engine directly.
  assert.deepEqual(await execute(`${keyScript} return [key('ArrowRight'), key('ArrowRight')];`), [true, true]);
  assert.match(await painted(), /, C1$/);
  assert.equal(await execute(`${keyScript} return key('ArrowDown');`), true);
  assert.match(await painted(), /, C2$/);
  assert.equal(await execute(`${keyScript} return key('Enter');`), true);
  assert.match(await painted(), /, C3$/);
  assert.equal(await execute(`${keyScript} return key('Tab');`), true);
  assert.match(await painted(), /, D3$/);

  // Shift+Arrow extends the range, but the active cell does not move.
  assert.equal(await execute(`${keyScript} return key('ArrowRight', { shiftKey: true });`), true);
  assert.match(await painted(), /, D3$/);
  // Shift+Enter traverses the column backward, so the active cell moves up.
  assert.equal(await execute(`${keyScript} return key('Enter', { shiftKey: true });`), true);
  assert.match(await painted(), /, D2$/);

  // Ctrl+Arrow moves to a data edge; Ctrl+Home returns to A1.
  assert.equal(await execute(`${keyScript} ${primary} return key('ArrowRight', primary);`), true);
  assert.match(await painted(), /, XFD2$/);
  assert.equal(await execute(`${keyScript} ${primary} return key('ArrowDown', primary);`), true);
  assert.match(await painted(), /, XFD1048576$/);
  assert.equal(await execute(`${keyScript} ${primary} return key('Home', primary);`), true);
  assert.match(await painted(), /, A1$/);
  assert.equal(await execute(`${keyScript} return key('PageDown');`), true);
  assert.equal(await execute(`${keyScript} ${primary} return key('Home', primary);`), true);
  assert.match(await painted(), /, A1$/);

  // Plain End moves to the used edge of the row. Ctrl+End moves to the used
  // range corner.
  assert.equal(await execute(`${keyScript} return key('End');`), true);
  assert.match(await painted(), /, C1$/);
  assert.equal(await execute(`${keyScript} ${primary} return key('End', primary);`), true);
  assert.match(await painted(), /, C5$/);
  assert.equal(await execute(`${keyScript} ${primary} return key('Home', primary);`), true);
  assert.match(await painted(), /, A1$/);

  // Alt+Arrow selects the next and previous sheet.
  assert.equal(await execute(`${keyScript} return key('ArrowDown', { altKey: true });`), true);
  await delay(50);
  assert.equal(await selectedTab(), "Sheet2");
  assert.match(await painted(), /, A1$/);
  assert.equal(await execute(`${keyScript} return key('ArrowUp', { altKey: true });`), true);
  await delay(50);
  assert.equal(await selectedTab(), "Sheet1");

  // IME, composition, and another focused element keep their keys.
  assert.equal(await execute(`${keyScript} return key('ArrowDown', { isComposing: true });`), false);
  assert.match(await painted(), /, A1$/);
  assert.equal(await execute(`${keyScript}
    canvas.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
    const consumed = key('ArrowDown');
    canvas.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true }));
    return consumed;`), false);
  assert.match(await painted(), /, A1$/);
  assert.equal(await execute(`${keyScript}
    const input = document.createElement('input'); document.body.append(input); input.focus();
    const consumed = key('ArrowDown'); input.remove(); canvas.focus(); return consumed;`), false);
  assert.match(await painted(), /, A1$/);

  // Ctrl+Shift+Arrow is outside the webapp navigation bindings.
  assert.equal(await execute(`${keyScript} ${primary} return key('ArrowRight', { ...primary, shiftKey: true });`), false);
  assert.match(await painted(), /, A1$/);

  // Navigation is synchronous. A scheduled paint survives focus loss.
  assert.equal(await execute(`${keyScript} const consumed = key('ArrowRight'); canvas.blur(); return consumed;`), true);
  assert.match(await painted(), /, B1$/);
  await execute("document.querySelector('canvas').focus()");

  // A pointer selects the cell under the committed frame.
  assert.equal(await execute(`${keyScript}
    canvas.dispatchEvent(new PointerEvent('pointerdown', { clientX: 120, clientY: 120, button: 0, bubbles: true, cancelable: true }));
    return true;`), true);
  await delay(50);
  assert.doesNotMatch(await painted(), /, A1$/);

  // Wheel input applies a pixel scroll and repaints the grid. The listener
  // must be non-passive so preventDefault stops the page scroll.
  const canvasState = async () => command(`/session/${session}/execute/async`, {
    script: "const done = arguments[0]; requestAnimationFrame(() => requestAnimationFrame(() => done(document.querySelector('canvas').toDataURL())));",
    args: [],
  });
  const beforeWheel = await canvasState();
  assert.equal(
    await execute(`${keyScript} return canvas.dispatchEvent(new WheelEvent('wheel', { deltaY: 30, deltaMode: 0, bubbles: true, cancelable: true }));`),
    false,
    "wheel input must be canceled so the page does not scroll",
  );
  await delay(100);
  assert.notEqual(await canvasState(), beforeWheel, "wheel input must repaint the grid");

  // A wheel event after a key command must keep that order. The key handler
  // reconciles navigation before the wheel changes the pixel position.
  await execute(`${keyScript} ${primary} return key('Home', primary);`);
  await painted();
  await execute(`${keyScript} return key('ArrowRight');`);
  await painted();
  const b1 = await canvasState();
  await execute(`${keyScript} ${primary} return key('Home', primary);`);
  await painted();
  await execute(`${keyScript}
    const consumed = key('ArrowRight');
    canvas.dispatchEvent(new WheelEvent('wheel', { deltaY: 30, deltaMode: 0, bubbles: true, cancelable: true }));
    return consumed;`);
  await delay(100);
  assert.notEqual(
    await canvasState(),
    b1,
    "a wheel after navigation must not be replaced by the reveal",
  );

  // Replacing the model frees the old one. Resize listeners must use the
  // current model, not the one captured when the host started.
  await executeWithArgs(`
    const bytes = Uint8Array.from(atob(arguments[0]), (value) => value.charCodeAt(0));
    const input = document.querySelector('.file-input');
    const transfer = new DataTransfer();
    transfer.items.add(new File([bytes], 'cf_tests.xlsx', { type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' }));
    input.files = transfer.files;
    input.dispatchEvent(new Event('change', { bubbles: true }));
    return true;`, [fixture]);
  await waitFor(() => execute("return document.querySelector('.workbook-name').textContent === 'cf_tests.xlsx';"));
  await command(`/session/${session}/window/rect`, { width: 760, height: 560 });
  await delay(400);
  assert.equal(await execute("return document.querySelector('.error-banner')?.textContent ?? null;"), null);
  await execute("document.querySelector('canvas').focus()");
  const beforeResize = await painted();
  assert.equal(await execute(`${keyScript} return key('ArrowRight');`), true);
  assert.notEqual(await painted(), beforeResize);
  assert.equal(await execute("return document.querySelector('.error-banner')?.textContent ?? null;"), null);

  console.log("React navigation route: engine keys, modifiers, sheet switch, IME, focus, pointer, wheel, and workbook reload pass.");
} finally {
  if (session) await fetch(`${driverUrl}/session/${session}`, { method: "DELETE" }).catch(() => {});
  for (const child of children) child.kill();
}
