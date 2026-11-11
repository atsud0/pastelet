(() => {
  let theme = "system";
  try {
    const saved = localStorage.getItem("pastelet-theme");
    if (["light", "dark"].includes(saved)) theme = saved;
  } catch {}
  document.documentElement.dataset.theme = theme;
  document.documentElement.dataset.js = "true";
  document.addEventListener("DOMContentLoaded", initialize);
})();

function initialize() {
  let theme = document.documentElement.dataset.theme;
  const themeButtons = document.querySelectorAll("[data-theme-choice]");
  const updateTheme = () => {
    document.documentElement.dataset.theme = theme;
    themeButtons.forEach(button => button.setAttribute("aria-pressed", String(button.dataset.themeChoice === theme)));
  };
  themeButtons.forEach(button => button.addEventListener("click", () => {
    theme = button.dataset.themeChoice;
    try { localStorage.setItem("pastelet-theme", theme); } catch {}
    updateTheme();
  }));
  updateTheme();
  initializeCreateForm();
  initializeShareLink(document, window);
  initializeReaderMenu(document, window);
  for (const event of ["htmx:sendError", "htmx:timeout"]) {
    document.body.addEventListener(event, () => {
      const feedback = document.getElementById("form-feedback");
      if (feedback) {
        feedback.textContent = "Connection failed";
        feedback.classList.add("form-error");
      }
    });
  }
  const copyButton = document.getElementById("copy-paste");
  if (copyButton?.dataset.burned === "true") {
    const bytes = Uint8Array.from(atob(copyButton.dataset.raw), character => character.charCodeAt(0));
    const url = URL.createObjectURL(new Blob([bytes], { type: "text/plain;charset=utf-8" }));
    const raw = document.querySelector('a[aria-label="Raw"]');
    const download = document.querySelector('a[aria-label="Download"]');
    if (raw) { raw.href = url; raw.target = "_blank"; raw.rel = "noopener"; }
    if (download) { download.href = url; download.download = download.dataset.filename; }
  }
  copyButton?.addEventListener("click", async () => {
    try {
      const bytes = Uint8Array.from(atob(copyButton.dataset.raw), character => character.charCodeAt(0));
      await navigator.clipboard.writeText(new TextDecoder("utf-8", { ignoreBOM: true }).decode(bytes));
      copyButton.title = "Copied";
      copyButton.setAttribute("aria-label", "Copied");
    } catch {
      copyButton.title = "Copy failed";
      copyButton.setAttribute("aria-label", "Copy failed");
    }
    const feedback = document.getElementById("action-feedback");
    if (feedback) feedback.textContent = copyButton.title;
  });

  const toggle = document.getElementById("view-toggle");
  const source = document.getElementById("pastelet-source");
  const rendered = document.getElementById("pastelet-rendered");
  if (!toggle || !source || !rendered) return;

  const showSource = enabled => {
    source.hidden = !enabled;
    rendered.hidden = enabled;
    toggle.setAttribute("aria-pressed", String(enabled));
    toggle.setAttribute("aria-label", enabled ? "Rendered" : "Source");
    toggle.title = enabled ? "Rendered" : "Source";
  };
  const followLineAnchor = () => {
    if (!/^#L[1-9][0-9]*$/.test(location.hash)) return;
    const target = document.getElementById(location.hash.slice(1));
    if (!target) return;
    showSource(true);
    target.scrollIntoView({ block: "center" });
  };
  toggle.addEventListener("click", () => showSource(source.hidden));
  window.addEventListener("hashchange", followLineAnchor);
  followLineAnchor();
}

function initializeShareLink(document, window) {
  const button = document.getElementById("copy-share-link");
  if (!button) return;
  const url = button.dataset.shareUrl;
  const feedback = document.createElement("div");
  feedback.className = "share-feedback";
  feedback.setAttribute("role", "status");
  document.body.append(feedback);
  let timer;
  button.hidden = false;
  button.addEventListener("click", async () => {
    let copied = false;
    if (window.navigator.clipboard?.writeText) {
      try {
        await window.navigator.clipboard.writeText(url);
        copied = true;
      } catch {}
    }
    if (!copied) {
      const field = document.createElement("input");
      field.className = "sr-only";
      field.readOnly = true;
      field.value = url;
      const previous = document.activeElement;
      document.body.append(field);
      try {
        field.focus({ preventScroll: true });
        field.select();
        copied = document.execCommand("copy");
      } catch {} finally {
        field.remove();
        previous?.focus({ preventScroll: true });
      }
    }
    window.clearTimeout(timer);
    if (copied) {
      feedback.textContent = "链接已复制";
      timer = window.setTimeout(() => { feedback.textContent = ""; }, 1800);
    } else {
      feedback.textContent = "";
      window.prompt("请手动复制链接", url);
    }
  });
}

function initializeReaderMenu(document, window) {
  const tools = document.getElementById("reader-tools");
  if (!tools) return;
  const toggle = document.getElementById("reader-menu-toggle");
  const menu = document.getElementById("reader-menu");
  const copy = document.getElementById("copy-paste");
  const download = tools.querySelector(".reader-download");
  const theme = tools.querySelector(".reader-theme");
  const mobile = window.matchMedia("(max-width: 767px)");
  const items = () => Array.from(menu.querySelectorAll("a[href], button:not(:disabled)"));
  const close = (restore = false) => {
    menu.hidden = true;
    toggle.setAttribute("aria-expanded", "false");
    if (restore) toggle.focus();
  };
  const open = (last = false) => {
    menu.hidden = false;
    toggle.setAttribute("aria-expanded", "true");
    const controls = items();
    controls[last ? controls.length - 1 : 0]?.focus();
  };
  const layout = () => {
    const focused = tools.contains(document.activeElement);
    if (mobile.matches) {
      menu.prepend(...[copy, download].filter(Boolean));
      menu.append(theme);
    } else {
      if (copy) tools.insertBefore(copy, toggle);
      tools.insertBefore(download, toggle);
      tools.append(theme);
    }
    toggle.hidden = items().length === 0;
    close(focused);
    if (focused && toggle.hidden) download.focus();
  };
  toggle.addEventListener("click", () => menu.hidden ? open() : close(true));
  toggle.addEventListener("keydown", event => {
    if (event.key === "Escape" && !menu.hidden) { event.preventDefault(); close(true); return; }
    if (!["ArrowDown", "ArrowUp"].includes(event.key)) return;
    event.preventDefault();
    open(event.key === "ArrowUp");
  });
  menu.addEventListener("keydown", event => {
    if (event.key === "Escape") {
      event.preventDefault();
      close(true);
      return;
    }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const controls = items();
    const current = controls.indexOf(document.activeElement);
    const index = event.key === "Home" ? 0 : event.key === "End" ? controls.length - 1
      : (current + (event.key === "ArrowDown" ? 1 : -1) + controls.length) % controls.length;
    controls[index]?.focus();
  });
  menu.addEventListener("click", event => {
    if (event.target.closest("a, button")) close(true);
  });
  tools.addEventListener("focusout", event => {
    if (event.relatedTarget && !tools.contains(event.relatedTarget)) close();
  });
  document.addEventListener("click", event => { if (!tools.contains(event.target)) close(); });
  mobile.addEventListener("change", layout);
  layout();
}

function initializeCreateForm() {
  const form = document.getElementById("create-paste");
  if (!form) return;
  const settings = document.getElementById("create-settings");
  const advanced = document.getElementById("advanced-fields");
  const settingsButton = document.getElementById("settings-toggle");
  const moreButton = document.getElementById("more-toggle");
  const mobile = window.matchMedia("(max-width: 767px)");
  const medium = window.matchMedia("(min-width: 768px) and (max-width: 1099px)");
  const setOpen = (button, panel, open) => {
    button.setAttribute("aria-expanded", String(open));
    panel.classList.toggle("is-open", open);
  };
  for (const [button, panel] of [[settingsButton, settings], [moreButton, advanced]]) {
    button.addEventListener("click", () => setOpen(button, panel, button.getAttribute("aria-expanded") !== "true"));
  }
  document.addEventListener("click", event => {
    if (mobile.matches && !settings.contains(event.target) && !settingsButton.contains(event.target)) setOpen(settingsButton, settings, false);
    if (medium.matches && !advanced.contains(event.target) && !moreButton.contains(event.target)) setOpen(moreButton, advanced, false);
  });
  const resetPanels = () => {
    setOpen(settingsButton, settings, false);
    setOpen(moreButton, advanced, false);
    if (mobile.matches && settings.contains(document.activeElement)) settingsButton.focus();
    else if (medium.matches && advanced.contains(document.activeElement)) moreButton.focus();
  };
  mobile.addEventListener("change", resetPanels);
  medium.addEventListener("change", resetPanels);
  document.addEventListener("keydown", event => {
    if (event.key === "Escape") {
      if (mobile.matches && settingsButton.getAttribute("aria-expanded") === "true") {
        setOpen(settingsButton, settings, false);
        settingsButton.focus();
      } else if (medium.matches && moreButton.getAttribute("aria-expanded") === "true") {
        setOpen(moreButton, advanced, false);
        moreButton.focus();
      }
    }
    if (event.key !== "Enter" || !event.ctrlKey || event.isComposing || event.repeat) return;
    event.preventDefault();
    if (form.classList.contains("htmx-request")) return;
    const submit = form.querySelector(mobile.matches ? ".mobile-create button" : ".desktop-submit");
    if (!submit.disabled) form.requestSubmit(submit);
  });
}
