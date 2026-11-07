function lineStarts(content) {
  const starts = [0];
  for (let offset = content.indexOf("\n"); offset !== -1; offset = content.indexOf("\n", offset + 1)) starts.push(offset + 1);
  return starts;
}

function lineAt(starts, offset) {
  let lower = 0;
  let upper = starts.length;
  while (lower < upper) {
    const middle = Math.floor((lower + upper) / 2);
    if (starts[middle] <= offset) lower = middle + 1;
    else upper = middle;
  }
  return Math.max(0, lower - 1);
}

function obviousProse(content) {
  const sample = content.slice(0, 16384);
  if (sample.length < 120 || /[{}\[\]();=<>`\\|#$]/.test(sample)) return false;
  const lines = sample.split("\n").map(line => line.trim()).filter(Boolean);
  return lines.length > 0 && lines.filter(line => line.length >= 60 && /[.!?。！？]/.test(line)).length * 3 >= lines.length * 2;
}

function editorMode(language, content) {
  const plain = language === "text" || (language === "auto" && obviousProse(content));
  return { plain, wrap: plain };
}

function visibleLines(count, scrollTop, height, padding, lineHeight) {
  const first = Math.floor(Math.max(0, scrollTop - padding) / lineHeight);
  const last = Math.ceil((scrollTop + height) / lineHeight);
  return { start: Math.max(0, Math.min(count - 1, first) - 2), end: Math.min(count, last + 3) };
}

class PasteEditor {
  constructor(document) {
    this.document = document;
    this.window = document.defaultView;
    this.input = document.getElementById("paste-content");
    this.gutter = document.getElementById("editor-gutter");
    this.activeLine = document.getElementById("editor-active-line");
    this.language = document.querySelector('select[name="language"]');
    this.content = null;
    this.starts = [0];
    this.frame = null;
    this.layoutNeeded = false;
    this.input.addEventListener("input", () => this.schedule(true));
    this.input.addEventListener("scroll", () => this.render());
    for (const event of ["click", "keyup", "select", "focus", "blur"]) this.input.addEventListener(event, () => this.schedule(false));
    document.addEventListener("selectionchange", () => {
      if (document.activeElement === this.input) this.schedule(false);
    });
    this.language.addEventListener("change", () => this.schedule(true));
    this.input.form.addEventListener("reset", () => this.schedule(true));
    this.observer = new this.window.ResizeObserver(() => this.schedule(true));
    this.observer.observe(this.input);
    document.fonts?.ready.then(() => this.schedule(true));
    this.schedule(true);
  }

  schedule(layout) {
    this.layoutNeeded ||= layout;
    if (this.frame !== null) return;
    this.frame = this.window.requestAnimationFrame(() => {
      this.frame = null;
      if (this.layoutNeeded) {
        this.layoutNeeded = false;
        this.layout();
      }
      this.render();
    });
  }

  layout() {
    if (this.content !== this.input.value) {
      this.content = this.input.value;
      this.starts = lineStarts(this.content);
    }
    const mode = this.input.dataset?.pdfAttached === "true" ? { plain: true, wrap: true } : editorMode(this.language.value, this.content);
    this.gutter.hidden = mode.plain;
    const wrap = mode.wrap ? "soft" : "off";
    if (this.input.wrap !== wrap) {
      const { selectionStart, selectionEnd, selectionDirection, scrollTop } = this.input;
      this.input.wrap = wrap;
      this.input.setSelectionRange(selectionStart, selectionEnd, selectionDirection);
      this.input.scrollTop = scrollTop;
      if (mode.wrap) this.input.scrollLeft = 0;
    }
    this.gutter.style.setProperty("--line-digits", String(Math.max(2, String(this.starts.length).length)));
    const style = this.window.getComputedStyle(this.input);
    this.lineHeight = parseFloat(style.lineHeight);
    this.padding = parseFloat(style.paddingTop);
  }

  render() {
    this.activeLine.hidden = this.gutter.hidden || this.document.activeElement !== this.input;
    if (this.gutter.hidden || !this.lineHeight) return;
    const caret = this.input.selectionDirection === "backward" ? this.input.selectionStart : this.input.selectionEnd;
    const active = lineAt(this.starts, caret);
    const scrollTop = this.input.scrollTop;
    this.activeLine.style.top = `${this.padding + active * this.lineHeight - scrollTop}px`;
    const range = visibleLines(this.starts.length, scrollTop, this.input.clientHeight, this.padding, this.lineHeight);
    const fragment = this.document.createDocumentFragment();
    for (let index = range.start; index < range.end; index++) {
      const number = this.document.createElement("div");
      number.className = index === active ? "editor-line is-active" : "editor-line";
      number.textContent = String(index + 1);
      number.style.top = `${this.padding + index * this.lineHeight - scrollTop}px`;
      fragment.append(number);
    }
    this.gutter.replaceChildren(fragment);
  }
}

function initializeReader(document) {
  const toggle = document.getElementById("reader-wrap");
  const source = document.querySelector("#rpaste-source .source-view");
  if (!toggle || !source) return;
  toggle.checked = source.classList.contains("is-wrapped");
  toggle.addEventListener("change", () => {
    source.classList.toggle("is-wrapped", toggle.checked);
    if (toggle.checked) source.scrollLeft = 0;
  });
}

const MAX_UPLOAD_BYTES = 1024 * 1024;
const UNSUPPORTED_FILE = "目前只支持文本和代码文件";
const BINARY_FILE = "这不是文本文件";
const FILE_LANGUAGES = {
  txt: "text", log: "text", md: "markdown", markdown: "markdown", rs: "Rust",
  py: "Python", pyw: "Python", js: "JavaScript", mjs: "JavaScript", cjs: "JavaScript",
  jsx: "JavaScript (Babel)", ts: "TypeScript", tsx: "TypeScriptReact", json: "JSON",
  html: "HTML", htm: "HTML", css: "CSS", scss: "SCSS", sass: "Sass", xml: "XML",
  svg: "XML", yml: "YAML", yaml: "YAML", toml: "TOML", sh: "Bash", bash: "Bash",
  c: "C", h: "C", cpp: "C++", cc: "C++", hpp: "C++", cs: "C#", java: "Java",
  go: "Go", rb: "Ruby", php: "PHP", sql: "SQL", lua: "Lua", swift: "Swift",
  kt: "Kotlin", kts: "Kotlin", r: "R", pl: "Perl", ex: "Elixir", exs: "Elixir",
  erl: "Erlang", hs: "Haskell", clj: "Clojure", scala: "Scala", vue: "Vue Component",
  ini: "INI", conf: "INI", diff: "Diff", patch: "Diff", tex: "LaTeX", ps1: "PowerShell",
};

function uploadLanguage(name, options) {
  const basename = name.toLowerCase();
  const extension = basename.includes(".") ? basename.split(".").pop() : "";
  const language = ({ dockerfile: "Dockerfile", makefile: "Makefile" })[basename] || FILE_LANGUAGES[extension];
  return Array.from(options).find(option => option.value.toLowerCase() === language?.toLowerCase())?.value || "auto";
}

function validateUpload(file) {
  if (file.size > MAX_UPLOAD_BYTES) throw new Error(`文件不能超过 ${MAX_UPLOAD_BYTES / 1024 / 1024} MB`);
  const extension = file.name.split(".").pop().toLowerCase();
  if (/^(pdf|docx?|xlsx?|pptx?|odt|ods|odp|rtf|epub|png|jpe?g|gif|webp|avif|heic|ico|bmp|tiff?|zip|rar|7z|gz|bz2|xz|tar|exe|dll|so|wasm|woff2?|ttf|otf|mp[34]|mov|avi|mkv|wav|ogg|flac|sqlite|db)$/.test(extension)
    || (/^(image|audio|video|font)\//.test(file.type) && file.type !== "image/svg+xml")
    || /pdf|zip|compressed|officedocument|msword|ms-excel|ms-powerpoint|opendocument/.test(file.type)) throw new Error(UNSUPPORTED_FILE);
}

function decodeUpload(buffer) {
  const bytes = new Uint8Array(buffer);
  const prefix = Array.from(bytes.slice(0, 16), byte => String.fromCharCode(byte)).join("");
  if (prefix.startsWith("%PDF-")) throw new Error(UNSUPPORTED_FILE);
  if (/^(\x89PNG|\xff\xd8\xff|GIF8|RIFF|PK\x03\x04|\x7fELF|MZ|\x1f\x8b|Rar!|7z\xbc\xaf|SQLite format)/.test(prefix)) throw new Error(BINARY_FILE);
  const clean = text => {
    if (/[\u0000-\u0008\u000b\u000e-\u001f\u007f-\u009f\ufffd]/u.test(text)) throw new Error(BINARY_FILE);
    return text.replace(/\r\n?/g, "\n");
  };
  const decode = encoding => new TextDecoder(encoding, { fatal: true }).decode(bytes);
  if (bytes[0] === 0xff && bytes[1] === 0xfe || bytes[0] === 0xfe && bytes[1] === 0xff) {
    let text;
    try { text = decode(bytes[0] === 0xff ? "utf-16le" : "utf-16be"); }
    catch { throw new Error("无法解码文件，请先转换为 UTF-8"); }
    return { text: clean(text), encoding: "UTF-16" };
  }
  if (bytes.some(byte => byte < 9 || byte === 11 || byte >= 14 && byte < 32 || byte === 127)) throw new Error(BINARY_FILE);
  let utf8;
  try { utf8 = decode("utf-8"); } catch {}
  if (utf8 !== undefined) return { text: clean(utf8), encoding: "UTF-8" };
  for (const encoding of ["gb18030", "windows-1252"]) {
    try {
      const text = clean(decode(encoding));
      const nonAscii = Array.from(text).filter(character => character.charCodeAt(0) > 127);
      const plausible = encoding === "gb18030"
        ? nonAscii.length >= 2 && nonAscii.filter(character => /[\u3400-\u9fff，。！？、；：“”‘’（）]/u.test(character)).length / nonAscii.length > .9
        : nonAscii.length > 0 && nonAscii.length / Math.max(1, text.length) < .2 && nonAscii.every(character => /[\p{L}\p{P}\p{Zs}]/u.test(character));
      if (plausible) return { text, encoding, guessed: true };
    } catch {}
  }
  throw new Error("无法识别文本编码，请先转换为 UTF-8");
}

class PasteUpload {
  constructor(document) {
    this.document = document;
    this.window = document.defaultView;
    this.input = document.getElementById("paste-content");
    this.title = document.querySelector('input[name="title"]');
    this.language = document.querySelector('select[name="language"]');
    this.picker = document.getElementById("upload-file");
    this.button = document.getElementById("upload-button");
    this.zone = document.getElementById("paste-editor");
    this.overlay = document.getElementById("upload-overlay");
    this.notice = document.getElementById("upload-notice");
    this.message = document.getElementById("upload-message");
    this.confirmation = document.getElementById("upload-confirmation");
    this.replace = document.getElementById("upload-replace");
    this.cancel = document.getElementById("upload-cancel");
    this.dismiss = document.getElementById("upload-dismiss");
    this.sequence = 0;
    this.depth = 0;
    this.button.hidden = false;
    this.button.addEventListener("click", () => this.pdf?.file ? this.pdf.toggleMenu() : this.picker.click());
    this.picker.addEventListener("change", () => {
      void this.importFiles(this.picker.files);
      this.picker.value = "";
    });
    this.replace.addEventListener("click", () => { if (this.pending?.apply) this.pending.apply(); else if (this.pending) this.apply(this.pending); });
    this.cancel.addEventListener("click", () => {
      this.pending = null;
      this.notice.close();
      this.button.focus();
    });
    this.dismiss.addEventListener("click", () => this.notice.close());
    this.notice.addEventListener("cancel", () => { this.pending = null; });
    this.notice.addEventListener("keydown", event => {
      if (event.ctrlKey && event.key === "Enter") { event.preventDefault(); event.stopPropagation(); }
    });
    this.input.form.addEventListener("reset", () => {
      this.sequence++;
      this.pending = null;
      this.notice.hidden = true;
      this.notice.close();
      this.confirmation.hidden = true;
    });
    const isFile = event => Array.from(event.dataTransfer?.types || []).includes("Files");
    this.zone.addEventListener("dragenter", event => {
      if (!isFile(event)) return;
      event.preventDefault();
      this.depth++;
      this.overlay.hidden = false;
    });
    this.zone.addEventListener("dragleave", () => {
      this.depth = Math.max(0, this.depth - 1);
      if (!this.depth) this.overlay.hidden = true;
    });
    this.zone.addEventListener("dragover", event => {
      if (!isFile(event)) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
    });
    this.zone.addEventListener("drop", event => {
      if (!isFile(event)) return;
      event.preventDefault();
      this.depth = 0;
      this.overlay.hidden = true;
      void this.importFiles(event.dataTransfer.files);
    });
    for (const type of ["dragover", "drop"]) document.addEventListener(type, event => {
      if (!isFile(event)) return;
      event.preventDefault();
      if (type === "drop") { this.depth = 0; this.overlay.hidden = true; }
    });
    if (this.window.PastePdfAttachment) this.pdf = new this.window.PastePdfAttachment(document, this);
  }

  show(message, confirm = false) {
    this.message.textContent = message;
    this.notice.hidden = false;
    this.confirmation.hidden = !confirm;
    this.dismiss.hidden = confirm;
    if (!this.notice.open) this.notice.showModal();
    if (!confirm) this.dismiss.focus();
  }

  async importFiles(files) {
    const file = files[0];
    if (!file) return;
    const sequence = ++this.sequence;
    this.pending = null;
    const warning = files.length > 1 ? "一次只能上传一个文件。" : "";
    try {
      if (this.pdf?.busy) throw new Error("PDF upload in progress");
      if (this.pdf?.accepts(file)) {
        await this.pdf.attach(file, warning, () => sequence === this.sequence);
        return;
      }
      validateUpload(file);
      this.notice.close();
      const buffer = await file.arrayBuffer();
      if (sequence !== this.sequence) return;
      const decoded = decodeUpload(buffer);
      const pending = { file, ...decoded, warning };
      if (this.input.value.length || decoded.guessed) {
        this.pending = pending;
        const question = this.input.value.length ? "替换现有内容吗？" : "导入文件吗？";
        this.show(`${warning}${decoded.guessed ? `编码推测为 ${decoded.encoding}。` : ""}${question}`, true);
        this.replace.textContent = this.input.value.length ? "替换" : "导入";
        this.cancel.focus();
      } else this.apply(pending);
    } catch (error) {
      const known = error instanceof Error && ([UNSUPPORTED_FILE, BINARY_FILE].includes(error.message) || /^(文件不能超过|无法解码|无法识别)/.test(error.message));
      if (sequence === this.sequence) this.show(`${warning}${known ? error.message : "无法读取文件，请重试"}`);
    }
  }

  apply(pending) {
    this.pdf?.detach();
    this.pending = null;
    this.input.value = pending.text;
    if (!this.title.value.trim()) this.title.value = Array.from(pending.file.name).slice(0, 200).join("");
    this.language.value = uploadLanguage(pending.file.name, this.language.options);
    this.input.dispatchEvent(new this.window.Event("input", { bubbles: true }));
    this.language.dispatchEvent(new this.window.Event("change", { bubbles: true }));
    this.input.focus();
    this.input.setSelectionRange(0, 0);
    this.input.scrollTop = 0;
    this.input.scrollLeft = 0;
    this.show(`${pending.warning}已导入${pending.encoding === "UTF-8" ? "" : ` · ${pending.encoding}`}`);
  }
}

if (typeof document !== "undefined") {
  document.addEventListener("DOMContentLoaded", () => {
    if (document.getElementById("paste-content")) {
      new PasteEditor(document);
      new PasteUpload(document);
    }
    initializeReader(document);
  });
}
if (typeof module !== "undefined") module.exports = { lineStarts, lineAt, obviousProse, editorMode, visibleLines, PasteEditor, initializeReader, MAX_UPLOAD_BYTES, validateUpload, decodeUpload, uploadLanguage, PasteUpload };
