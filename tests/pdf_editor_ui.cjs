const { test } = require("node:test");
const assert = require("node:assert/strict");
const { PastePdfAttachment, isPdfFile } = require("../static/pdf-editor.js");

function harness() {
  const elements = new Map();
  const element = id => {
    if (!elements.has(id)) elements.set(id, {
      id, value: "", disabled: false, hidden: true, dataset: {}, attributes: {}, listeners: {}, captures: {}, textContent: "Create",
      addEventListener(type, callback, capture) { this.listeners[type] = callback; this.captures[type] = capture; },
      setAttribute(name, value) { this.attributes[name] = value; },
      removeAttribute(name) { delete this.attributes[name]; },
      contains(target) { return target === this; },
      focus() { this.focused = true; },
      dispatchEvent(event) { this.listeners[event.type]?.(event); },
      setSelectionRange(start, end, direction) { this.selectionStart = start; this.selectionEnd = end; this.selectionDirection = direction; },
      querySelector(selector) { return element(selector); },
    });
    return elements.get(id);
  };
  const input = element("input");
  input.form = element("form");
  input.selectionStart = 2; input.selectionEnd = 4; input.selectionDirection = "forward";
  input.scrollTop = 100; input.scrollLeft = 20;
  const fields = ["title", "expires_in", "password", "burn_after_read"].map(element);
  fields.namedItem = element;
  input.form.elements = fields;
  input.form.querySelectorAll = () => [element("desktop-label"), element("mobile-label")];
  const requests = [];
  const locations = [];
  class Request {
    constructor() { this.listeners = {}; this.upload = element(`upload-${requests.length}`); requests.push(this); }
    open(method, path) { this.method = method; this.path = path; }
    setRequestHeader() {}
    addEventListener(type, callback) { this.listeners[type] = callback; }
    send(data) { this.data = data; }
  }
  class Data { constructor() { this.fields = new Map(); } append(key, value) { this.fields.set(key, value); } }
  const window = { Event, XMLHttpRequest: Request, FormData: Data, location: { assign: path => locations.push(path) } };
  const document = { defaultView: window, getElementById: element, addEventListener() {} };
  const messages = [];
  const owner = { input, title: element("title"), language: element("language"), button: element("button"), picker: element("picker"),
    replace: element("replace"), cancel: element("cancel"), show: (...message) => messages.push(message) };
  owner.picker.dataset.maxPdfBytes = String(10 * 1024 * 1024);
  owner.language.value = "Python";
  const pdf = new PastePdfAttachment(document, owner);
  return { pdf, owner, input, element, requests, locations, messages };
}

function file(name = "paper.pdf", content = "%PDF-1.7", size = 100) {
  return { name, size, type: "application/pdf", reads: 0,
    slice(start, end) { this.reads++; return { arrayBuffer: async () => new TextEncoder().encode(content.slice(start, end)).buffer }; } };
}

test("PDF detection is separate from existing text and code import", () => {
  assert.equal(isPdfFile(file()), true);
  assert.equal(isPdfFile({ name: "doc.PDF", type: "" }), true);
  for (const name of ["test.py", "index.html", "data.txt"]) assert.equal(isPdfFile({ name, type: "text/plain" }), false);
});

test("PDF limit is checked before reading, and header spoofing does not replace editor", async () => {
  const { pdf, input, messages } = harness();
  input.value = "original";
  const large = file("large.pdf", "%PDF-", 10 * 1024 * 1024 + 1);
  await pdf.attach(large, "", () => true);
  assert.equal(large.reads, 0);
  assert.match(messages.at(-1)[0], /10 MB/);
  await pdf.attach(file("fake.pdf", "NOT-PDF"), "", () => true);
  assert.match(messages.at(-1)[0], /不是有效的 PDF/);
  assert.equal(input.value, "original");
  assert.equal(pdf.file, null);
});

test("attachment uses existing icon, readonly placeholder, no line numbers and restores draft", async () => {
  const { pdf, owner, input, element } = harness();
  input.value = "print('original')";
  await pdf.attach(file(), "", () => true);
  assert.equal(pdf.file, null);
  owner.pending.apply();
  assert.equal(input.readOnly, true);
  assert.equal(input.dataset.pdfAttached, "true");
  assert.equal(owner.language.disabled, true);
  assert.equal(owner.title.value, "paper.pdf");
  assert.match(input.value, /已附加 PDF：paper.pdf/);
  assert.match(owner.button.attributes["data-tooltip"], /paper.pdf.*KB/);
  assert.equal(element("[data-upload-icon]").hidden, true);
  assert.equal(element("[data-pdf-icon]").hidden, false);
  pdf.toggleMenu();
  assert.equal(element("pdf-file-menu").hidden, false);
  element("pdf-remove").listeners.click();
  assert.equal(pdf.file, null);
  assert.equal(input.value, "print('original')");
  assert.equal(input.selectionStart, 2);
  assert.equal(input.selectionEnd, 4);
  assert.equal(input.scrollTop, 100);
  assert.equal(input.readOnly, false);
  assert.equal(owner.language.disabled, false);
  assert.equal(owner.title.value, "");
  assert.equal(element("[data-pdf-icon]").hidden, true);
});

test("user title is never overwritten, stale file reads do not attach", async () => {
  const { pdf, owner } = harness();
  owner.title.value = "User title";
  await pdf.attach(file(), "", () => false);
  assert.equal(pdf.file, null);
  await pdf.attach(file(), "", () => true);
  assert.equal(owner.title.value, "User title");
  pdf.detach();
  assert.equal(owner.title.value, "User title");
});

test("progress and server/network failures preserve selected PDF and settings for retry", async () => {
  const { pdf, owner, input, element, requests, messages } = harness();
  const selected = file();
  await pdf.attach(selected, "", () => true);
  owner.title.value = "Report";
  element("password").value = "secret";
  element("expires_in").value = "1h";
  element("burn_after_read").checked = true;
  pdf.upload();
  const request = requests[0];
  assert.equal(request.data.fields.get("file"), selected);
  assert.equal(request.data.fields.get("password"), "secret");
  assert.equal(request.data.fields.get("expires_in"), "1h");
  assert.equal(request.data.fields.get("burn_after_read"), "true");
  assert.equal(request.data.fields.has("content"), false);
  request.upload.listeners.progress({ lengthComputable: true, loaded: 40, total: 100 });
  assert.equal(element("desktop-label").textContent, "上传 40%");
  request.status = 413; request.responseText = "{}"; request.listeners.load();
  assert.equal(pdf.busy, false);
  assert.equal(pdf.file, selected);
  assert.equal(input.readOnly, true);
  assert.equal(owner.language.disabled, true);
  assert.equal(element("password").disabled, false);
  assert.match(messages.at(-1)[0], /过大/);
  pdf.upload(); requests[1].listeners.error();
  assert.equal(pdf.file, selected);
  assert.match(messages.at(-1)[0], /文件已保留/);
  assert.equal(element("desktop-label").textContent, "Create");
});

test("PDF submit intercepts HTMX, success redirects only to a local paste ID", async () => {
  const { pdf, input, requests, locations } = harness();
  const event = { preventDefault() { this.prevented = true; }, stopImmediatePropagation() { this.stopped = true; } };
  input.form.listeners.submit(event);
  assert.equal(event.prevented, undefined);
  await pdf.attach(file(), "", () => true);
  input.form.listeners.submit(event);
  assert.equal(event.prevented, true);
  assert.equal(event.stopped, true);
  assert.equal(input.form.captures.submit, true);
  requests[0].status = 201; requests[0].responseText = '{"url":"https://evil.example"}'; requests[0].listeners.load();
  assert.deepEqual(locations, []);
  pdf.upload(); requests[1].status = 201; requests[1].responseText = '{"url":"/Ab12Cd34"}'; requests[1].listeners.load();
  assert.deepEqual(locations, ["/Ab12Cd34"]);
});
