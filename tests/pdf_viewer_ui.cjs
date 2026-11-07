const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

async function harness({ burned = false, fail = false, count = 2, sizes = {}, brokenPage, slow = false } = {}) {
  const elements = new Map();
  const nodes = [];
  const create = () => {
    const node = {
      dataset: {}, listeners: {}, style: {}, attributes: {}, children: [], value: "1", disabled: true,
      clientWidth: 640, clientHeight: 700, scrollTop: 0, textContent: "正在加载 PDF…",
      addEventListener(type, callback) { this.listeners[type] = callback; },
      setAttribute(name, value) { this.attributes[name] = value; }, getContext() { return {}; },
      append(child) { this.children.push(child); child.parent = this; },
      remove() { this.parent.children = this.parent.children.filter(child => child !== this); },
    };
    nodes.push(node);
    return node;
  };
  const element = id => {
    if (!elements.has(id)) elements.set(id, create());
    return elements.get(id);
  };
  const original = readFileSync(`${__dirname}/fixtures/two-pages.pdf`);
  const viewer = element("pdf-viewer");
  viewer.dataset = { pdf: original.toString("base64"), burned: String(burned), filename: "报告.pdf" };
  const document = { getElementById: element, querySelector: () => element("download"), createElement: create };
  const events = {};
  const window = { devicePixelRatio: 2, addEventListener: (type, callback) => { events[type] = callback; },
    requestAnimationFrame: callback => setImmediate(callback), cancelAnimationFrame: clearImmediate };
  let disconnected = false;
  let resize;
  class Observer { constructor(callback) { resize = callback; } observe() {} disconnect() { disconnected = true; } }
  const urls = [];
  const revoked = [];
  const url = { createObjectURL: blob => { urls.push(blob); return "blob:local"; }, revokeObjectURL: value => revoked.push(value) };
  const renders = [];
  let options;
  let destroyed = false;
  const cleaned = [];
  let cancelled = 0;
  const pdf = { numPages: count, getPage: async number => ({
    getViewport: ({ scale }) => ({ width: (sizes[number]?.[0] || 600) * scale, height: (sizes[number]?.[1] || 800) * scale }),
    cleanup() { cleaned.push(number); },
    render: settings => {
      renders.push({ number, ...settings });
      let reject;
      let timer;
      const promise = number === brokenPage ? Promise.reject(new Error("bad page")) : slow
        ? new Promise((resolve, failure) => { reject = failure; timer = setTimeout(resolve, 10); }) : Promise.resolve();
      return { promise, cancel() { cancelled++; clearTimeout(timer); reject?.(Object.assign(new Error("cancelled"), { name: "RenderingCancelledException" })); } };
    },
  }) };
  const library = { GlobalWorkerOptions: {}, getDocument: input => {
    options = input;
    return { promise: fail ? Promise.reject(new Error("invalid PDF")) : Promise.resolve(pdf), destroy: async () => { destroyed = true; } };
  } };
  const source = readFileSync(`${__dirname}/../static/pdf-viewer.mjs`, "utf8")
    .replace('await import("/static/pdfjs/pdf.min.mjs")', "await loadPdfjs()");
  await new AsyncFunction("document", "window", "ResizeObserver", "URL", "Blob", "atob", "loadPdfjs", source)
    (document, window, Observer, url, Blob, atob, async () => library);
  return { element, document, nodes, original, options, library, renders, events, urls, revoked, resize, cleaned,
    cancelled: () => cancelled, destroyed: () => destroyed, disconnected: () => disconnected };
}

const settle = () => new Promise(resolve => setImmediate(resolve));
const sheets = result => result.element("pdf-pages").children;
const canvases = result => sheets(result).flatMap(sheet => sheet.children.filter(child => child.attributes.role === "img"));
const topOf = (result, index) => 16 + sheets(result).slice(0, index).reduce((total, sheet) => total + parseFloat(sheet.style.height) + 16, 0);
const scrollTo = async (result, index, fraction = 0) => {
  result.element("pdf-scroll").scrollTop = topOf(result, index) + parseFloat(sheets(result)[index].style.height) * fraction;
  result.element("pdf-scroll").listeners.scroll();
  await settle();
  await settle();
};

test("PDF snapshot renders and page/zoom controls work without a second file fetch", async () => {
  const result = await harness();
  const { element, original, options, library, renders } = result;
  assert.deepEqual(Buffer.from(options.data), original);
  assert.equal(element("pdf-viewer").dataset.pdf, undefined);
  assert.equal(library.GlobalWorkerOptions.workerSrc, "/static/pdfjs/pdf.worker.min.mjs");
  assert.equal(options.enableXfa, false);
  assert.equal(renders[0].number, 1);
  assert.equal(sheets(result).length, 2);
  assert.deepEqual(renders.map(render => render.number), [1, 2]);
  assert.equal(element("pdf-previous").disabled, true);
  assert.equal(element("pdf-next").disabled, false);
  element("pdf-next").listeners.click();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(element("pdf-page").value, "2");
  assert.ok(element("pdf-scroll").scrollTop > 0);
  assert.equal(element("pdf-next").disabled, true);
  const width = renders.at(-1).viewport.width;
  element("pdf-zoom-in").listeners.click();
  await new Promise(resolve => setImmediate(resolve));
  assert.ok(renders.at(-1).viewport.width > width);
  element("pdf-page").value = "999";
  element("pdf-page").listeners.change();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(element("pdf-page").value, "2");
});

test("scrolling advances the current page, renders nearby sheets and releases distant canvases", async () => {
  const result = await harness({ count: 1000 });
  assert.equal(sheets(result).length, 1000);
  assert.ok(result.renders.length <= 3);
  const initial = canvases(result);
  await scrollTo(result, 499);
  assert.equal(result.element("pdf-page").value, "500");
  assert.ok(result.renders.some(render => render.number === 500));
  assert.ok(canvases(result).length <= 8);
  assert.ok(initial.every(canvas => canvas.width === 0 && canvas.height === 0));
  assert.ok(result.renders.length < 10);
  await scrollTo(result, 0);
  assert.equal(result.element("pdf-page").value, "1");
  assert.equal(result.element("pdf-previous").disabled, true);
  assert.equal(result.renders.filter(render => render.number === 1).length, 2);
});

test("zoom and responsive resize preserve the reading position, including mixed page sizes", async () => {
  const result = await harness({ count: 20, sizes: { 5: [800, 400], 6: [400, 1000] } });
  await scrollTo(result, 5, .35);
  const fraction = () => (result.element("pdf-scroll").scrollTop - topOf(result, 5)) / parseFloat(sheets(result)[5].style.height);
  const before = fraction();
  result.element("pdf-zoom-in").listeners.click();
  await settle();
  assert.ok(Math.abs(fraction() - before) < .001);
  assert.equal(result.element("pdf-page").value, "6");
  result.element("pdf-scroll").clientWidth = 360;
  result.resize();
  await new Promise(resolve => setTimeout(resolve, 170));
  assert.ok(Math.abs(fraction() - before) < .001);
  assert.equal(parseFloat(sheets(result)[5].style.width), (360 - 32) * 1.25);
  assert.ok(canvases(result).every(canvas => canvas.width * canvas.height <= 3 * 1024 * 1024));
});

test("rapid scroll and zoom discard stale renders, and a broken page does not block subsequent pages", async () => {
  const result = await harness({ count: 12, slow: true, brokenPage: 7 });
  await scrollTo(result, 5);
  result.element("pdf-zoom-in").listeners.click();
  await scrollTo(result, 8);
  await new Promise(resolve => setTimeout(resolve, 100));
  assert.ok(result.cancelled() > 0);
  assert.equal(result.element("pdf-page").value, "9");
  assert.ok(canvases(result).some(canvas => canvas.attributes["aria-label"].includes("第 9 页")));
  await scrollTo(result, 6);
  await new Promise(resolve => setTimeout(resolve, 100));
  assert.match(sheets(result)[6].children[0].textContent, /无法渲染/);
  assert.ok(canvases(result).some(canvas => canvas.attributes["aria-label"].includes("第 8 页")));
});

test("burned PDF download is original local bytes with title, cleanup destroys loading task", async () => {
  const result = await harness({ burned: true });
  assert.equal(result.element("download").href, "blob:local");
  assert.equal(result.element("download").download, "报告.pdf");
  assert.deepEqual(Buffer.from(await result.urls[0].arrayBuffer()), result.original);
  result.events.pagehide({ persisted: true });
  assert.equal(result.destroyed(), false);
  const ready = canvases(result);
  result.events.pagehide({ persisted: false });
  assert.equal(result.destroyed(), true);
  assert.equal(result.disconnected(), true);
  assert.deepEqual(result.revoked, ["blob:local"]);
  assert.ok(ready.every(canvas => canvas.width === 0));
  const rendered = result.renders.length;
  await scrollTo(result, 1);
  assert.equal(result.renders.length, rendered);
});

test("scrolling does not overwrite a page number while the user is typing", async () => {
  const result = await harness({ count: 10 });
  const input = result.element("pdf-page");
  result.document.activeElement = input;
  input.value = "8";
  await scrollTo(result, 3);
  assert.equal(input.value, "8");
  input.listeners.change();
  await settle();
  assert.ok(Math.abs(result.element("pdf-scroll").scrollTop - (topOf(result, 7) - 16)) < 1);
  input.listeners.blur();
  assert.equal(input.value, "8");
});

test("corrupt PDF fails visibly without a broken text action; burned download remains available", async () => {
  const result = await harness({ burned: true, fail: true });
  assert.match(result.element("pdf-status").textContent, /无法打开/);
  assert.equal(result.element("download").href, "blob:local");
});
