const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { runInNewContext } = require("node:vm");

const script = readFileSync(`${__dirname}/../static/app.js`, "utf8");
const url = "https://paste.example/Ab12Cd34Ef56Gh78Ij90Kl";
const content = "# Only the original text\r\n";

function harness({ clipboard = "available", legacy = false, hasText = false } = {}) {
  const writes = [];
  const prompts = [];
  const nodes = [];
  const elements = {};
  let initialize;
  let timer;
  const makeNode = () => ({
    dataset: {}, listeners: {}, textContent: "", hidden: true,
    addEventListener(event, handler) { this.listeners[event] = handler; },
    setAttribute(name, value) { this[name] = value; },
    focus() { document.activeElement = this; },
    select() { this.selected = true; },
    remove() { nodes.splice(nodes.indexOf(this), 1); },
  });
  const document = {
    documentElement: { dataset: {} },
    body: { append(node) { nodes.push(node); }, addEventListener() {} },
    addEventListener(event, handler) { if (event === "DOMContentLoaded") initialize = handler; },
    getElementById: id => elements[id],
    querySelectorAll: () => [],
    createElement: makeNode,
    execCommand(command) {
      assert.equal(command, "copy");
      assert.equal(document.activeElement.readOnly, true);
      assert.equal(document.activeElement.selected, true);
      if (legacy) writes.push(document.activeElement.value);
      return legacy;
    },
  };
  const share = elements["copy-share-link"] = makeNode();
  share.dataset.shareUrl = url;
  document.activeElement = share;
  if (hasText) {
    elements["copy-paste"] = makeNode();
    elements["copy-paste"].dataset.raw = Buffer.from(content).toString("base64");
  }
  const navigator = clipboard === "unavailable" ? {} : {
    clipboard: { async writeText(value) {
      if (clipboard === "denied") throw new Error("Permission denied");
      writes.push(value);
    } },
  };
  const window = {
    navigator, setTimeout(callback) { timer = callback; return 1; }, clearTimeout() {},
    prompt(message, value) { prompts.push(value); },
    location: { href: "http://tailnet-node:8007/download/Ab12Cd34Ef56Gh78Ij90Kl?private=1#L12" },
  };
  runInNewContext(script, { document, window, navigator, TextDecoder, atob });
  initialize();
  return { share, elements, document, writes, prompts, nodes, expire: () => timer() };
}

test("share and copy-text actions write different values; sharing needs no network or form", async () => {
  const { share, elements, writes, nodes, expire } = harness({ hasText: true });
  assert.equal(share.hidden, false);
  await share.listeners.click();
  await elements["copy-paste"].listeners.click();
  assert.deepEqual(writes, [url, content]);
  assert.equal(nodes[0].role, "status");
  assert.equal(nodes[0].textContent, "链接已复制");
  expire();
  assert.equal(nodes[0].textContent, "");
});

test("Tailnet HTTP can copy a protected link without a clipboard API or paste content", async () => {
  const { share, document, writes, prompts, nodes } = harness({ clipboard: "unavailable", legacy: true });
  await share.listeners.click();
  assert.deepEqual(writes, [url]);
  assert.deepEqual(prompts, []);
  assert.equal(nodes.length, 1);
  assert.equal(document.activeElement, share);
});

test("if clipboard writes are blocked, manual copy offers only the canonical link", async () => {
  const { share, prompts, writes, nodes } = harness({ clipboard: "denied" });
  await share.listeners.click();
  assert.deepEqual(writes, []);
  assert.deepEqual(prompts, [url]);
  assert.equal(nodes[0].textContent, "");
});
