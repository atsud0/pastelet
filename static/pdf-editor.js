function isPdfFile(file) {
  return /\.pdf$/i.test(file.name) || file.type === "application/pdf";
}

function pdfFileSize(bytes) {
  return bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

class PastePdfAttachment {
  constructor(document, owner) {
    this.document = document;
    this.window = document.defaultView;
    this.owner = owner;
    this.input = owner.input;
    this.form = this.input.form;
    this.menu = document.getElementById("pdf-file-menu");
    this.remove = document.getElementById("pdf-remove");
    this.maxBytes = Number(owner.picker.dataset.maxPdfBytes);
    this.file = null;
    this.busy = false;
    this.remove.addEventListener("click", () => { this.detach(); owner.button.focus(); });
    this.menu.addEventListener("keydown", event => {
      if (event.key === "Escape") { event.preventDefault(); this.closeMenu(); owner.button.focus(); }
    });
    owner.button.addEventListener("keydown", event => {
      if (this.file && event.key === "ArrowDown") { event.preventDefault(); this.menu.hidden = false; owner.button.setAttribute("aria-expanded", "true"); this.remove.focus(); }
      if (event.key === "Escape") this.closeMenu();
    });
    document.addEventListener("click", event => {
      if (!this.menu.contains(event.target) && !owner.button.contains(event.target)) this.closeMenu();
    });
    this.menu.addEventListener("focusout", event => {
      if (event.relatedTarget && !this.menu.contains(event.relatedTarget) && event.relatedTarget !== owner.button) this.closeMenu();
    });
    this.form.addEventListener("submit", event => {
      if (!this.file) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (!this.busy) this.upload();
    }, true);
    this.form.addEventListener("reset", event => {
      if (this.busy) { event.preventDefault(); return; }
      this.detach();
    });
  }

  accepts(file) { return isPdfFile(file); }

  async attach(file, warning, current) {
    if (file.size > this.maxBytes) { this.owner.show(`${warning}PDF 不能超过 ${this.maxBytes / 1024 / 1024} MB`); return; }
    const header = new Uint8Array(await file.slice(0, 5).arrayBuffer());
    if (!current()) return;
    if (String.fromCharCode(...header) !== "%PDF-") { this.owner.show(`${warning}这不是有效的 PDF 文件`); return; }
    const apply = () => {
      this.owner.pending = null;
      this.setFile(file);
      this.owner.show(`${warning}已附加 PDF`);
    };
    if (this.file || this.input.value.length) {
      this.owner.pending = { apply };
      this.owner.replace.textContent = "附加";
      this.owner.show(`${warning}${this.file ? "替换已附加的 PDF 吗？" : "附加 PDF 并暂存现有文本吗？"}`, true);
      this.owner.cancel.focus();
    } else apply();
  }

  setFile(file) {
    if (!this.file) {
      this.saved = { content: this.input.value, start: this.input.selectionStart, end: this.input.selectionEnd,
        direction: this.input.selectionDirection, scrollTop: this.input.scrollTop, scrollLeft: this.input.scrollLeft,
        language: this.owner.language.value, title: this.owner.title.value };
    }
    this.file = file;
    if (!this.owner.title.value.trim()) {
      this.autoTitle = Array.from(file.name).slice(0, 200).join("");
      this.owner.title.value = this.autoTitle;
    }
    this.input.readOnly = true;
    this.input.dataset.pdfAttached = "true";
    this.input.value = `已附加 PDF：${file.name}，点击顶部图标可移除`;
    this.owner.language.disabled = true;
    const label = `已附加 PDF：${file.name} · ${pdfFileSize(file.size)}`;
    this.owner.button.setAttribute("aria-label", label);
    this.owner.button.setAttribute("title", label);
    this.owner.button.setAttribute("data-tooltip", label);
    this.owner.button.setAttribute("aria-controls", "pdf-file-menu");
    this.owner.button.setAttribute("aria-expanded", "false");
    this.owner.button.querySelector("[data-upload-icon]").hidden = true;
    this.owner.button.querySelector("[data-pdf-icon]").hidden = false;
    this.input.dispatchEvent(new this.window.Event("input", { bubbles: true }));
  }

  closeMenu() {
    this.menu.hidden = true;
    if (this.file) this.owner.button.setAttribute("aria-expanded", "false");
  }

  toggleMenu() {
    if (this.busy) return;
    this.menu.hidden = !this.menu.hidden;
    this.owner.button.setAttribute("aria-expanded", String(!this.menu.hidden));
    if (!this.menu.hidden) this.remove.focus();
  }

  detach() {
    if (!this.file || this.busy) return;
    this.file = null;
    this.input.readOnly = false;
    delete this.input.dataset.pdfAttached;
    this.input.value = this.saved.content;
    this.input.setSelectionRange(this.saved.start, this.saved.end, this.saved.direction);
    this.input.scrollTop = this.saved.scrollTop;
    this.input.scrollLeft = this.saved.scrollLeft;
    this.owner.language.disabled = false;
    this.owner.language.value = this.saved.language;
    if (this.autoTitle && this.owner.title.value === this.autoTitle) this.owner.title.value = this.saved.title;
    this.autoTitle = null;
    this.closeMenu();
    for (const name of ["data-tooltip", "aria-controls", "aria-expanded"]) this.owner.button.removeAttribute(name);
    this.owner.button.setAttribute("aria-label", "上传文件");
    this.owner.button.setAttribute("title", "上传文件");
    this.owner.button.querySelector("[data-upload-icon]").hidden = false;
    this.owner.button.querySelector("[data-pdf-icon]").hidden = true;
    this.input.dispatchEvent(new this.window.Event("input", { bubbles: true }));
    this.owner.language.dispatchEvent(new this.window.Event("change", { bubbles: true }));
  }

  upload() {
    const data = new this.window.FormData();
    data.append("file", this.file, this.file.name);
    for (const name of ["title", "expires_in", "password"]) data.append(name, this.form.elements.namedItem(name).value);
    data.append("burn_after_read", String(this.form.elements.namedItem("burn_after_read").checked));
    const request = new this.window.XMLHttpRequest();
    const controls = Array.from(this.form.elements).map(control => [control, control.disabled]);
    const labels = Array.from(this.form.querySelectorAll(".create-submit > span")).map(label => [label, label.textContent]);
    this.busy = true;
    this.closeMenu();
    this.form.setAttribute("aria-busy", "true");
    this.owner.button.setAttribute("aria-label", "正在上传 PDF");
    for (const [control] of controls) control.disabled = true;
    const progress = value => { for (const [label] of labels) label.textContent = value; };
    const finish = message => {
      this.busy = false;
      this.form.removeAttribute("aria-busy");
      for (const [control, disabled] of controls) control.disabled = disabled;
      for (const [label, text] of labels) label.textContent = text;
      this.owner.button.setAttribute("aria-label", `已附加 PDF：${this.file.name}`);
      if (message) this.owner.show(message);
    };
    request.open("POST", "/paste/pdf");
    request.setRequestHeader("Accept", "application/json");
    request.timeout = 120000;
    request.upload.addEventListener("progress", event => {
      if (event.lengthComputable) progress(`上传 ${Math.min(99, Math.floor(event.loaded / event.total * 100))}%`);
    });
    request.upload.addEventListener("load", () => progress("处理中…"));
    request.addEventListener("load", () => {
      let result;
      try { result = JSON.parse(request.responseText); } catch {}
      if (request.status === 201 && /^\/[A-Za-z0-9]{8}$/.test(result?.url)) {
        progress("100%");
        this.window.location.assign(result.url);
        return;
      }
      finish(request.status === 413 ? "PDF 过大，上传失败" : `上传失败：${result?.error?.message || "请重试"}`);
    });
    for (const event of ["error", "abort", "timeout"]) request.addEventListener(event, () => finish("上传未完成，请检查网络后重试；文件已保留"));
    progress("上传 0%");
    try { request.send(data); } catch { finish("无法上传，请重试；文件已保留"); }
  }
}

if (typeof window !== "undefined") window.PastePdfAttachment = PastePdfAttachment;
if (typeof module !== "undefined") module.exports = { isPdfFile, pdfFileSize, PastePdfAttachment };
