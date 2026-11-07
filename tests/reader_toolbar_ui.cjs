const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { runInNewContext } = require("node:vm");

function harness(width, pdf = false) {
  const elements = {};
  const document = {
    documentElement: { dataset: {} }, listeners: {}, activeElement: null,
    getElementById: id => elements[id],
    addEventListener(type, callback) { this.listeners[type] = callback; },
  };
  function element(id, control = false) {
    const node = {
      id, control, hidden: false, children: [], parent: null, listeners: {}, attributes: {},
      addEventListener(type, callback) { this.listeners[type] = callback; },
      setAttribute(key, value) { this.attributes[key] = value; },
      contains(target) { return target === this || this.children.some(child => child.contains(target)); },
      focus() { document.activeElement = this; },
      remove() { if (this.parent) this.parent.children.splice(this.parent.children.indexOf(this), 1); },
      append(...nodes) { for (const child of nodes) { child.remove(); child.parent = this; this.children.push(child); } },
      prepend(...nodes) { for (const child of [...nodes].reverse()) { child.remove(); child.parent = this; this.children.unshift(child); } },
      insertBefore(child, sibling) { child.remove(); child.parent = this; this.children.splice(this.children.indexOf(sibling), 0, child); },
      querySelector(selector) { return elements[selector === ".reader-download" ? "download" : "theme"]; },
      querySelectorAll() { return this.children.flatMap(child => child.control ? [child] : child.querySelectorAll()); },
      closest() { return this.control ? this : null; },
    };
    elements[id] = node;
    return node;
  }
  const tools = element("reader-tools");
  const toggle = element("reader-menu-toggle", true);
  const menu = element("reader-menu");
  const copy = element("copy-paste", true);
  const download = element("download", true);
  const source = element("source", true);
  const raw = element("raw", true);
  const fork = element("fork", true);
  const theme = element("theme");
  theme.append(element("system", true), element("light", true), element("dark", true));
  menu.append(source, copy, raw, download, fork, theme);
  if (pdf) {
    for (const control of [source, copy, raw, fork]) control.remove();
    delete elements["copy-paste"];
  }
  tools.append(toggle, menu);
  const media = { matches: width < 768, addEventListener(type, callback) { this.change = callback; } };
  const window = { matchMedia: () => media };
  runInNewContext(readFileSync(`${__dirname}/../static/app.js`, "utf8") + "\ninitializeReaderMenu(document, window);", { document, window });
  const key = (node, key) => {
    const event = { key, preventDefault() { this.prevented = true; } };
    node.listeners.keydown(event);
    return event;
  };
  return { elements, tools, toggle, menu, copy, download, theme, media, document, key };
}

test("desktop exposes copy/download/theme while keeping uncommon actions in dropdown", () => {
  const { tools, menu, copy, download, theme, toggle } = harness(1440);
  assert.deepEqual(tools.children.map(child => child.id), ["copy-paste", "download", "reader-menu-toggle", "reader-menu", "theme"]);
  assert.equal(menu.hidden, true);
  assert.equal(toggle.attributes["aria-expanded"], "false");
  assert.equal(copy.parent, tools);
  assert.equal(download.parent, tools);
  assert.equal(theme.parent, tools);
  assert.deepEqual(menu.children.map(child => child.id), ["source", "raw", "fork"]);
});

test("PDF has no copy or empty desktop menu, while mobile keeps download and theme", () => {
  const { toggle, menu, download, theme, media } = harness(1440, true);
  assert.equal(toggle.hidden, true);
  media.matches = true;
  media.change();
  assert.equal(toggle.hidden, false);
  assert.equal(download.parent, menu);
  assert.equal(theme.parent, menu);
  assert.equal(menu.children.length, 2);
});

test("mobile keeps only overflow trigger and reuses original action elements", () => {
  const { tools, menu, copy, download, theme, media } = harness(375);
  assert.deepEqual(tools.children.map(child => child.id), ["reader-menu-toggle", "reader-menu"]);
  assert.equal(copy.parent, menu);
  assert.equal(download.parent, menu);
  assert.equal(theme.parent, menu);
  copy.marker = "burn snapshot and click handlers remain attached";
  media.matches = false;
  media.change();
  assert.equal(copy.parent, tools);
  media.matches = true;
  media.change();
  assert.equal(copy.parent, menu);
  assert.equal(copy.marker, "burn snapshot and click handlers remain attached");
  assert.equal(menu.children.filter(child => child === copy).length, 1);
});

test("keyboard arrows, Home/End and Escape navigate and restore focus", () => {
  const { toggle, menu, document, elements, key } = harness(900);
  assert.equal(key(toggle, "ArrowDown").prevented, true);
  assert.equal(menu.hidden, false);
  assert.equal(document.activeElement, elements.source);
  key(menu, "ArrowDown");
  assert.equal(document.activeElement, elements.raw);
  key(menu, "End");
  assert.equal(document.activeElement, elements.fork);
  key(menu, "ArrowDown");
  assert.equal(document.activeElement, elements.source);
  key(menu, "ArrowUp");
  assert.equal(document.activeElement, elements.fork);
  key(menu, "Home");
  assert.equal(document.activeElement, elements.source);
  key(menu, "Escape");
  assert.equal(menu.hidden, true);
  assert.equal(document.activeElement, toggle);
});

test("click outside, Tab leaving, actions and breakpoint changes close disclosure", () => {
  const { toggle, menu, tools, document, elements, media } = harness(375);
  toggle.listeners.click();
  document.listeners.click({ target: {} });
  assert.equal(menu.hidden, true);
  toggle.listeners.click();
  tools.listeners.focusout({ relatedTarget: {} });
  assert.equal(menu.hidden, true);
  toggle.listeners.click();
  menu.listeners.click({ target: elements.dark });
  assert.equal(menu.hidden, true);
  assert.equal(document.activeElement, toggle);
  toggle.listeners.click();
  media.matches = false;
  media.change();
  assert.equal(menu.hidden, true);
  assert.equal(document.activeElement, toggle);
});
