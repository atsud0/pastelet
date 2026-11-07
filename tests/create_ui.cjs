const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { runInNewContext } = require("node:vm");

function harness(width) {
  const listeners = {};
  const elements = {};
  function element(id) {
    const classes = new Set();
    const attributes = { "aria-expanded": "false" };
    return elements[id] = {
      disabled: false,
      listeners: {},
      classList: {
        contains: value => classes.has(value),
        toggle: (value, enabled) => enabled ? classes.add(value) : classes.delete(value),
      },
      setAttribute: (key, value) => attributes[key] = value,
      getAttribute: key => attributes[key],
      addEventListener(event, callback) { this.listeners[event] = callback; },
      contains(target) { return target === this; },
      focus() { document.activeElement = this; },
    };
  }
  for (const id of ["create-paste", "create-settings", "advanced-fields", "settings-toggle", "more-toggle", "desktop", "mobile"]) element(id);
  const submissions = [];
  const form = elements["create-paste"];
  form.querySelector = selector => elements[selector.includes("mobile") ? "mobile" : "desktop"];
  form.requestSubmit = button => submissions.push(button);
  const document = {
    documentElement: { dataset: {} },
    getElementById: id => elements[id],
    addEventListener: (event, callback) => listeners[event] = callback,
  };
  const media = [];
  const window = {
    matchMedia(query) {
      const match = { matches: query.includes("min-width") ? width >= 768 && width < 1100 : width < 768,
        addEventListener(event, callback) { this.change = callback; } };
      media.push(match);
      return match;
    },
  };
  runInNewContext(readFileSync(`${__dirname}/../static/app.js`, "utf8") + "\ninitializeCreateForm();", { document, window });
  return { elements, listeners, submissions, media, document };
}

test("Ctrl+Enter chooses the visible submit button and ignores repeats, IME and pending requests", () => {
  for (const width of [375, 900, 1440]) {
    const state = harness(width);
    let prevented = 0;
    const event = { key: "Enter", ctrlKey: true, preventDefault() { prevented++; } };
    state.listeners.keydown(event);
    assert.equal(state.submissions[0], state.elements[width < 768 ? "mobile" : "desktop"]);
    assert.equal(prevented, 1);
    for (const extra of [{ repeat: true }, { isComposing: true }, { ctrlKey: false }]) state.listeners.keydown({ ...event, ...extra });
    state.elements["create-paste"].classList.toggle("htmx-request", true);
    state.listeners.keydown(event);
    state.elements["create-paste"].classList.toggle("htmx-request", false);
    state.submissions[0].disabled = true;
    state.listeners.keydown(event);
    assert.equal(state.submissions.length, 1);
  }
});

test("Settings and More close with Escape or outside clicks without closing on internal clicks", () => {
  for (const [width, buttonId, panelId] of [[375, "settings-toggle", "create-settings"], [900, "more-toggle", "advanced-fields"]]) {
    const state = harness(width);
    const button = state.elements[buttonId];
    const panel = state.elements[panelId];
    button.listeners.click();
    assert.equal(button.getAttribute("aria-expanded"), "true");
    state.listeners.click({ target: panel });
    assert.equal(panel.classList.contains("is-open"), true);
    state.listeners.keydown({ key: "Escape" });
    assert.equal(button.getAttribute("aria-expanded"), "false");
    assert.equal(state.document.activeElement, button);
    button.listeners.click();
    state.listeners.click({ target: {} });
    assert.equal(panel.classList.contains("is-open"), false);
  }
});

test("Breakpoint changes reset disclosures and recover focus from collapsed controls", () => {
  const state = harness(900);
  state.elements["more-toggle"].listeners.click();
  state.document.activeElement = state.elements["create-settings"];
  state.media[0].matches = true;
  state.media[1].matches = false;
  state.media[0].change();
  assert.equal(state.elements["more-toggle"].getAttribute("aria-expanded"), "false");
  assert.equal(state.document.activeElement, state.elements["settings-toggle"]);
});
