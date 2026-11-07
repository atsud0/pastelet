const { test } = require("node:test");
const assert = require("node:assert/strict");
const { lineStarts, lineAt, editorMode, visibleLines, PasteEditor, initializeReader } = require("../static/editor.js");

test("logical lines include blank and final empty lines, and use textarea UTF-16 selection offsets", () => {
  assert.deepEqual(lineStarts(""), [0]);
  assert.deepEqual(lineStarts("a\n\n🙂\n"), [0, 2, 3, 6]);
  const starts = lineStarts("a\n\n🙂\n");
  assert.equal(lineAt(starts, 0), 0);
  assert.equal(lineAt(starts, 1), 0);
  assert.equal(lineAt(starts, 2), 1);
  assert.equal(lineAt(starts, 5), 2);
  assert.equal(lineAt(starts, 6), 3);
});

test("editor language defaults and prose detection are conservative", () => {
  const prose = "这是一段普通文章，包含完整的句子和正常的中文标点。".repeat(8);
  const english = "This is a long paragraph with complete sentences describing a quiet afternoon and the view from the window. ".repeat(3);
  for (const content of [prose, english]) assert.deepEqual(editorMode("auto", content, null), { plain: true, wrap: true });
  for (const content of ["", "Hello world", `${prose}\nconst value = 2;`, "x".repeat(500)]) {
    assert.deepEqual(editorMode("auto", content, null), { plain: false, wrap: false });
  }
  for (const language of ["Rust", "Python", "markdown"]) {
    assert.deepEqual(editorMode(language, prose, null), { plain: false, wrap: false });
  }
  assert.deepEqual(editorMode("text", "fn main() {}", null), { plain: true, wrap: true });
});

test("large documents only render the visible gutter", () => {
  const starts = lineStarts("line\n".repeat(100000));
  assert.equal(starts.length, 100001);
  const range = visibleLines(starts.length, 1400000, 560, 20, 28, null);
  assert.ok(range.start > 49990);
  assert.ok(range.end - range.start < 30);
});

function harness() {
  const frames = [];
  const elements = {};
  const listeners = {};
  function element() {
    return {
      style: { setProperty(key, value) { this[key] = value; } },
      children: [], listeners: {}, hidden: false, checked: false,
      append(child) { this.children.push(child); },
      replaceChildren(fragment) { this.children = fragment?.children ?? []; },
      addEventListener(event, callback) { this.listeners[event] = callback; },
    };
  }
  for (const id of ["paste-content", "paste-editor", "editor-gutter", "editor-active-line", "language"]) elements[id] = element();
  const input = elements["paste-content"];
  Object.assign(input, { value: "", selectionStart: 0, selectionEnd: 0, selectionDirection: "none", wrap: "off", scrollTop: 0, scrollLeft: 0, clientHeight: 560, clientWidth: 300, form: element(),
    setSelectionRange(start, end, direction) { this.selectionStart = start; this.selectionEnd = end; this.selectionDirection = direction; },
  });
  elements.language.value = "auto";
  const document = {
    getElementById: id => elements[id],
    querySelector: () => elements.language,
    createElement: element,
    createDocumentFragment: element,
    addEventListener: (event, callback) => listeners[event] = callback,
    defaultView: {
      requestAnimationFrame(callback) { frames.push(callback); return frames.length; },
      ResizeObserver: class { constructor(callback) { this.callback = callback; } observe() {} },
      getComputedStyle: () => ({ lineHeight: "24.05px", paddingTop: "24px", paddingLeft: "20px", paddingRight: "20px" }),
    },
  };
  const editor = new PasteEditor(document);
  function flush() { while (frames.length) frames.shift()(); }
  flush();
  return { input, elements, editor, flush, frames, document, listeners };
}

test("paste updates once per frame, widens gutter, tracks scrolling and caret without copying numbers into value", () => {
  const state = harness();
  const content = "const value = 1;\n".repeat(5000);
  state.input.value = content;
  state.input.selectionStart = state.input.selectionEnd = content.length;
  state.input.listeners.input();
  state.input.listeners.input();
  assert.equal(state.frames.length, 1);
  state.flush();
  const gutter = state.elements["editor-gutter"];
  assert.equal(gutter.style["--line-digits"], "4");
  assert.ok(gutter.children.length < 30);
  state.input.scrollTop = 5000 * 24.05;
  state.input.listeners.scroll();
  const active = gutter.children.find(child => child.className.includes("is-active"));
  assert.equal(active.textContent, "5001");
  assert.equal(active.style.top, "24px");
  assert.equal(state.input.value, content);
  state.input.value = "";
  state.input.selectionStart = state.input.selectionEnd = 0;
  state.input.listeners.input();
  state.flush();
  assert.equal(gutter.style["--line-digits"], "2");
  assert.equal(gutter.children.length, 1);
});

test("automatic editor wrapping preserves content and selection", () => {
  const state = harness();
  state.input.value = "a long line with characters that wrap\nsecond";
  state.input.selectionStart = 2;
  state.input.selectionEnd = 18;
  state.input.selectionDirection = "backward";
  state.input.listeners.input();
  state.elements.language.value = "text";
  state.elements.language.listeners.change();
  state.flush();
  assert.equal(state.input.wrap, "soft");
  assert.deepEqual([state.input.selectionStart, state.input.selectionEnd, state.input.selectionDirection], [2, 18, "backward"]);
  assert.equal(state.elements["editor-gutter"].hidden, true);
  state.elements.language.value = "Rust";
  state.elements.language.listeners.change();
  state.flush();
  assert.equal(state.input.wrap, "off");
  assert.equal(state.elements["editor-gutter"].hidden, false);
  assert.equal(state.elements["editor-gutter"].children.length, 2);
  assert.equal(state.input.value, "a long line with characters that wrap\nsecond");
});

test("automatic language changes and reset restore defaults, and cursor selection updates the active line", () => {
  const state = harness();
  state.elements.language.value = "text";
  state.elements.language.listeners.change();
  state.flush();
  assert.equal(state.input.wrap, "soft");
  state.elements.language.value = "Rust";
  state.elements.language.listeners.change();
  state.flush();
  assert.equal(state.input.wrap, "off");
  state.input.value = "first\nsecond\nthird";
  state.input.listeners.input();
  state.flush();
  state.document.activeElement = state.input;
  state.input.selectionStart = state.input.selectionEnd = 6;
  state.listeners.selectionchange();
  state.flush();
  assert.equal(state.elements["editor-gutter"].children.find(child => child.className.includes("is-active")).textContent, "2");
  state.input.form.listeners.reset();
  state.flush();
  assert.equal(state.input.wrap, "off");
});

test("reader wrap changes only layout, retaining all content, line numbers and anchors", () => {
  for (const initialWrap of [true, false]) {
    const classes = new Set(initialWrap ? ["is-wrapped"] : []);
    const children = [{ id: "L1", number: 1 }, { text: "one long line" }, { id: "L2", number: 2 }, { text: "second" }];
    const source = {
      children,
      scrollLeft: 120,
      classList: {
        contains: name => classes.has(name),
        toggle: (name, enabled) => enabled ? classes.add(name) : classes.delete(name),
      },
    };
    const toggle = { addEventListener(event, callback) { this.change = callback; } };
    initializeReader({ getElementById: () => toggle, querySelector: () => source });
    assert.equal(toggle.checked, initialWrap);
    for (const wrap of [true, false, true]) {
      toggle.checked = wrap;
      toggle.change();
      assert.equal(classes.has("is-wrapped"), wrap);
      assert.equal(source.children, children);
      assert.equal(source.children[2].id, "L2");
    }
    assert.equal(source.scrollLeft, 0);
  }
  initializeReader({ getElementById: () => null, querySelector: () => null });
});

test("full-width active line follows fractional scrolling and hides on blur or plain-text mode", () => {
  const state = harness();
  const highlight = state.elements["editor-active-line"];
  assert.equal(highlight.hidden, true);
  state.input.value = "first\nsecond\nthird";
  state.input.selectionStart = 6;
  state.input.selectionEnd = 17;
  state.input.selectionDirection = "backward";
  state.document.activeElement = state.input;
  state.input.listeners.input();
  state.input.listeners.focus();
  state.flush();
  assert.equal(highlight.hidden, false);
  assert.equal(highlight.style.top, "48.05px");
  state.input.scrollTop = 11.5;
  state.input.listeners.scroll();
  assert.ok(Math.abs(parseFloat(highlight.style.top) - 36.55) < 0.000001);
  const number = state.elements["editor-gutter"].children.find(child => child.textContent === "2");
  assert.equal(highlight.style.top, number.style.top);
  state.document.activeElement = null;
  state.input.listeners.blur();
  state.flush();
  assert.equal(highlight.hidden, true);
  state.document.activeElement = state.input;
  state.elements.language.value = "text";
  state.elements.language.listeners.change();
  state.flush();
  assert.equal(highlight.hidden, true);
  assert.equal(state.input.value, "first\nsecond\nthird");
});
