const { test } = require("node:test");
const assert = require("node:assert/strict");
const { MAX_UPLOAD_BYTES, validateUpload, decodeUpload, uploadLanguage, PasteUpload } = require("../static/editor.js");

function fixture() {
  const elements = new Map();
  const element = id => {
    if (!elements.has(id)) elements.set(id, {
      value: "", hidden: true, textContent: "", listeners: {}, events: [],
      addEventListener(type, callback) { this.listeners[type] = callback; },
      dispatchEvent(event) { this.events.push(event.type); this.listeners[event.type]?.(event); },
      focus() { this.focused = true; },
      showModal() { this.open = true; },
      close() { this.open = false; },
      click() { this.clicked = true; this.listeners.click?.(); },
      setSelectionRange(start, end) { this.selectionStart = start; this.selectionEnd = end; },
    });
    return elements.get(id);
  };
  const input = element("paste-content");
  input.form = element("form");
  const language = element("language");
  language.options = ["auto", "text", "markdown", "Python", "JavaScript", "HTML", "Rust"].map(value => ({ value }));
  language.value = "auto";
  const document = {
    defaultView: { Event },
    getElementById: element,
    querySelector: selector => element(selector.includes("title") ? "title" : "language"),
    addEventListener: (type, callback) => element("document").addEventListener(type, callback),
  };
  return { upload: new PasteUpload(document), element, input, language, title: element("title") };
}

function file(name, text, extra = {}) {
  const bytes = typeof text === "string" ? new TextEncoder().encode(text) : Uint8Array.from(text);
  return { name, type: "", size: bytes.length, reads: 0, async arrayBuffer() { this.reads++; return bytes.buffer; }, ...extra };
}

test("file size and unsupported types are checked before any reading", async () => {
  const { upload, input, element } = fixture();
  for (const rejected of [file("large.txt", "", { size: MAX_UPLOAD_BYTES + 1 }), file("book.PDF", "%PDF-"), file("image.txt", "", { type: "image/png" }), file("book.docx", "")]) {
    await upload.importFiles([rejected]);
    assert.equal(rejected.reads, 0);
    assert.equal(input.value, "");
    assert.match(element("upload-message").textContent, /文件不能超过|目前只支持/);
  }
  assert.doesNotThrow(() => validateUpload(file("text.txt", "", { size: MAX_UPLOAD_BYTES })));
});

test("UTF-8, BOM, CRLF and UTF-16 decode without HTML execution", () => {
  assert.equal(decodeUpload(new TextEncoder().encode("\ufeff<script>alert(1)</script>\r\n中文")).text, "<script>alert(1)</script>\n中文");
  assert.equal(decodeUpload(Uint8Array.from([0xff, 0xfe, 0x41, 0, 0x2d, 0x4e])).text, "A中");
  assert.equal(decodeUpload(Uint8Array.from([0xfe, 0xff, 0, 0x41, 0x4e, 0x2d])).text, "A中");
  assert.throws(() => decodeUpload(Uint8Array.from([0xff, 0xfe, 0x41])), /无法解码/);
});

test("legacy encoding inference is conservative and explicitly marked", () => {
  const chinese = decodeUpload(Uint8Array.from([0xd6, 0xd0, 0xce, 0xc4]));
  assert.equal(chinese.text, "中文");
  assert.equal(chinese.guessed, true);
  const western = decodeUpload(Uint8Array.from([...Buffer.from("A cup of caf"), 0xe9, 0x2e]));
  assert.equal(western.text, "A cup of café.");
  assert.equal(western.guessed, true);
  assert.throws(() => decodeUpload(Uint8Array.from([0xff, 0xfe, 0xff])), /无法解码/);
  assert.throws(() => decodeUpload(Uint8Array.from([0xff, 0xff, 0xff])), /无法识别/);
});

test("binary signatures, NUL and controls cannot enter editor even with text extension", async () => {
  const { upload, input, element } = fixture();
  for (const bytes of [[0x89, 0x50, 0x4e, 0x47], [0xff, 0xd8, 0xff], [65, 0, 66], [65, 1, 66], [...Buffer.from("GIF89a")], [...Buffer.from("PK\x03\x04")]]) {
    await upload.importFiles([file("fake.txt", bytes)]);
    assert.equal(input.value, "");
    assert.equal(element("upload-message").textContent, "这不是文本文件");
  }
  await upload.importFiles([file("fake.txt", "%PDF-1.7")]);
  assert.equal(element("upload-message").textContent, "目前只支持文本和代码文件");
});

test("known extensions choose actual select values, unknown resets Auto", () => {
  const { language } = fixture();
  for (const [name, expected] of [["HELLO.PY", "Python"], ["index.js", "JavaScript"], ["readme.md", "markdown"], ["file.txt", "text"], ["x.html", "HTML"], ["a.unknown", "auto"]]) {
    assert.equal(uploadLanguage(name, language.options), expected);
  }
});

test("import updates title, language, editor events and caret; HTML stays raw text", async () => {
  const { upload, input, title, language } = fixture();
  await upload.importFiles([file("index.html", "<script>bad()</script>")]);
  assert.equal(input.value, "<script>bad()</script>");
  assert.equal(title.value, "index.html");
  assert.equal(language.value, "HTML");
  assert.deepEqual(input.events, ["input"]);
  assert.deepEqual(language.events, ["change"]);
  assert.equal(input.selectionStart, 0);
});

test("nonempty content requires inline confirmation, cancellation keeps every field", async () => {
  const { upload, input, title, language, element } = fixture();
  input.value = "original";
  title.value = "My title";
  language.value = "Rust";
  await upload.importFiles([file("new.py", "print(1)")]);
  assert.equal(input.value, "original");
  assert.equal(element("upload-confirmation").hidden, false);
  assert.equal(element("upload-notice").open, true);
  assert.equal(element("upload-message").textContent, "替换现有内容吗？");
  element("upload-cancel").click();
  assert.equal(element("upload-notice").open, false);
  assert.equal(input.value, "original");
  assert.equal(title.value, "My title");
  assert.equal(language.value, "Rust");
  await upload.importFiles([file("new.py", "print(1)")]);
  element("upload-replace").click();
  assert.equal(input.value, "print(1)");
  assert.equal(title.value, "My title");
  assert.equal(language.value, "Python");
  assert.equal(element("upload-notice").open, true);
  element("upload-dismiss").click();
  assert.equal(element("upload-notice").open, false);
});

test("dialog Escape cancels pending replacement and Ctrl+Enter never creates", async () => {
  const { upload, input, element } = fixture();
  input.value = "keep";
  await upload.importFiles([file("new.txt", "replace")]);
  element("upload-notice").listeners.cancel();
  element("upload-replace").click();
  assert.equal(input.value, "keep");
  const event = { ctrlKey: true, key: "Enter", preventDefault() { this.prevented = true; }, stopPropagation() { this.stopped = true; } };
  element("upload-notice").listeners.keydown(event);
  assert.equal(event.prevented, true);
  assert.equal(event.stopped, true);
});

test("multiple files import only first and retain warning", async () => {
  const { upload, input, element } = fixture();
  const second = file("second.txt", "second");
  await upload.importFiles([file("first.txt", "first"), second]);
  assert.equal(input.value, "first");
  assert.equal(second.reads, 0);
  assert.match(element("upload-message").textContent, /一次只能上传一个文件/);
});

test("stale asynchronous reads cannot overwrite later imports or newly typed text", async () => {
  const { upload, input, element } = fixture();
  let resolve;
  const slow = file("slow.txt", "", { arrayBuffer: () => new Promise(done => { resolve = done; }) });
  const pending = upload.importFiles([slow]);
  await upload.importFiles([file("fast.txt", "fast")]);
  resolve(new TextEncoder().encode("slow").buffer);
  await pending;
  assert.equal(input.value, "fast");
  input.value = "";
  const typing = upload.importFiles([slow]);
  input.value = "typing";
  resolve(new TextEncoder().encode("slow").buffer);
  await typing;
  assert.equal(input.value, "typing");
  assert.equal(element("upload-confirmation").hidden, false);
});

test("read failures preserve content; reset cancels pending imports", async () => {
  const { upload, input, element } = fixture();
  input.value = "keep";
  await upload.importFiles([file("fail.txt", "", { arrayBuffer: async () => { throw new Error("denied"); } })]);
  assert.equal(input.value, "keep");
  assert.equal(element("upload-message").textContent, "无法读取文件，请重试");
  await upload.importFiles([file("other.txt", "other")]);
  input.form.dispatchEvent(new Event("reset"));
  element("upload-replace").click();
  assert.equal(input.value, "keep");
});

test("picker and nested drag events work without text-drop interference", () => {
  const { element } = fixture();
  element("upload-button").click();
  assert.equal(element("upload-file").clicked, true);
  const zone = element("paste-editor");
  const event = { dataTransfer: { types: ["Files"] }, preventDefault() { this.prevented = true; } };
  zone.listeners.dragenter(event);
  zone.listeners.dragenter(event);
  zone.listeners.dragleave();
  assert.equal(element("upload-overlay").hidden, false);
  zone.listeners.dragleave();
  assert.equal(element("upload-overlay").hidden, true);
  const text = { dataTransfer: { types: ["text/plain"] }, preventDefault() { throw new Error("must not intercept text"); } };
  zone.listeners.dragover(text);
});

test("large multiline imports update in a single input event", async () => {
  const { upload, input } = fixture();
  const text = "print('hello')\n".repeat(20000);
  await upload.importFiles([file("large.py", text)]);
  assert.equal(input.value, text);
  assert.deepEqual(input.events, ["input"]);
});

test("legacy guesses need explicit confirmation even in an empty editor", async () => {
  const { upload, input, element } = fixture();
  await upload.importFiles([file("chinese.txt", [0xd6, 0xd0, 0xce, 0xc4])]);
  assert.equal(input.value, "");
  assert.match(element("upload-message").textContent, /编码推测为 gb18030/);
  element("upload-replace").click();
  assert.equal(input.value, "中文");
});

test("picker resets its value for repeated selection and drop reads files", async () => {
  const { input, element } = fixture();
  const picker = element("upload-file");
  picker.value = "same.txt";
  picker.files = [file("same.txt", "picker")];
  picker.listeners.change();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(input.value, "picker");
  assert.equal(picker.value, "");
  input.value = "";
  const event = {
    dataTransfer: { types: ["Files"], files: [file("drop.py", "drop")] },
    preventDefault() { this.prevented = true; },
  };
  element("paste-editor").listeners.drop(event);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(event.prevented, true);
  assert.equal(input.value, "drop");
});

test("unrecognized extension resets language and filenames respect title limit", async () => {
  const { upload, title, language } = fixture();
  language.value = "Rust";
  await upload.importFiles([file(`${"文".repeat(210)}.unknown`, "text")]);
  assert.equal(language.value, "auto");
  assert.equal(Array.from(title.value).length, 200);
});
