const viewer = document.getElementById("pdf-viewer");

if (viewer) {
  const status = document.getElementById("pdf-status");
  const pages = document.getElementById("pdf-pages");
  const scroll = document.getElementById("pdf-scroll");
  const pageInput = document.getElementById("pdf-page");
  const previous = document.getElementById("pdf-previous");
  const next = document.getElementById("pdf-next");
  const zoomOut = document.getElementById("pdf-zoom-out");
  const zoomIn = document.getElementById("pdf-zoom-in");
  let downloadUrl;
  let task;
  try {
    const bytes = Uint8Array.from(atob(viewer.dataset.pdf), character => character.charCodeAt(0));
    delete viewer.dataset.pdf;
    if (viewer.dataset.burned === "true") {
      downloadUrl = URL.createObjectURL(new Blob([bytes], { type: "application/pdf" }));
      const download = document.querySelector(".reader-download");
      download.href = downloadUrl;
      download.download = viewer.dataset.filename;
    }
    const pdfjs = await import("/static/pdfjs/pdf.min.mjs");
    pdfjs.GlobalWorkerOptions.workerSrc = "/static/pdfjs/pdf.worker.min.mjs";
    task = pdfjs.getDocument({ data: bytes, cMapUrl: "/static/pdfjs/cmaps/", cMapPacked: true,
      standardFontDataUrl: "/static/pdfjs/standard_fonts/", wasmUrl: "/static/pdfjs/wasm/", iccUrl: "/static/pdfjs/iccs/",
      maxImageSize: 16 * 1024 * 1024, canvasMaxAreaInBytes: 32 * 1024 * 1024, enableXfa: false });
    task.onPassword = () => { status.textContent = "此 PDF 自身已加密，请下载后打开"; void task.destroy(); };
    const pdf = await task.promise;
    let pageNumber = 1;
    let zoom = 1;
    let revision = 0;
    let rendering;
    let pendingResize;
    let pendingScroll;
    let pumping = false;
    let stopped = false;
    let width = scroll.clientWidth;
    let wanted = new Set();
    const firstPage = await pdf.getPage(1);
    const initial = firstPage.getViewport({ scale: 1 });
    firstPage.cleanup();
    const sheets = Array.from({ length: pdf.numPages }, (_, index) => {
      const node = document.createElement("section");
      node.className = "pdf-sheet";
      node.setAttribute("aria-label", `PDF 第 ${index + 1} 页，共 ${pdf.numPages} 页`);
      const label = document.createElement("span");
      label.textContent = `第 ${index + 1} 页`;
      node.append(label);
      pages.append(node);
      return { number: index + 1, node, label, base: initial, top: 0, height: 0, canvas: null, failed: false };
    });
    const locate = position => {
      let low = 0;
      let high = sheets.length - 1;
      while (low < high) {
        const middle = Math.ceil((low + high) / 2);
        if (sheets[middle].top <= position) low = middle;
        else high = middle - 1;
      }
      return sheets[low];
    };
    const layout = () => {
      const anchor = locate(scroll.scrollTop);
      const fraction = anchor.height ? (scroll.scrollTop - anchor.top) / anchor.height : 0;
      let top = 16;
      for (const sheet of sheets) {
        sheet.top = top;
        sheet.width = Math.max(1, width - 32) * zoom;
        sheet.height = sheet.width * sheet.base.height / sheet.base.width;
        sheet.node.style.width = `${sheet.width}px`;
        sheet.node.style.height = `${sheet.height}px`;
        top += sheet.height + 16;
      }
      scroll.scrollTop = Math.max(0, anchor.top + fraction * anchor.height);
    };
    const release = sheet => {
      if (sheet.canvas) {
        sheet.canvas.remove();
        sheet.canvas.width = sheet.canvas.height = 0;
        sheet.canvas = null;
      }
    };
    pageInput.max = String(pdf.numPages);
    document.getElementById("pdf-total").textContent = `/ ${pdf.numPages}`;
    pageInput.disabled = false;
    const update = () => {
      pageNumber = locate(scroll.scrollTop + Math.min(100, scroll.clientHeight / 4)).number;
      if (document.activeElement !== pageInput) pageInput.value = String(pageNumber);
      previous.disabled = pageNumber <= 1;
      next.disabled = pageNumber >= pdf.numPages;
      zoomOut.disabled = zoom <= .5;
      zoomIn.disabled = zoom >= 3;
      const start = Math.max(1, locate(scroll.scrollTop).number - 1);
      const end = Math.min(pdf.numPages, locate(scroll.scrollTop + scroll.clientHeight).number + 1);
      wanted = new Set(Array.from({ length: end - start + 1 }, (_, index) => start + index)
        .sort((left, right) => Math.abs(left - pageNumber) - Math.abs(right - pageNumber)).slice(0, 8));
      for (const sheet of sheets) if (!wanted.has(sheet.number)) release(sheet);
      if (rendering && !wanted.has(rendering.number)) rendering.task.cancel();
    };
    const pump = async () => {
      if (pumping || stopped) return;
      pumping = true;
      try {
        while (!stopped) {
          const sheet = [...wanted].map(number => sheets[number - 1]).find(sheet => !sheet.canvas && !sheet.failed);
          if (!sheet) break;
          const current = revision;
          let page;
          let canvas;
          try {
            page = await pdf.getPage(sheet.number);
            if (stopped || current !== revision || !wanted.has(sheet.number)) continue;
            const base = page.getViewport({ scale: 1 });
            if (base.width !== sheet.base.width || base.height !== sheet.base.height) {
              sheet.base = base;
              layout();
              update();
            }
            const viewport = page.getViewport({ scale: sheet.width / base.width });
            const density = Math.min(window.devicePixelRatio || 1, 2, Math.sqrt(3 * 1024 * 1024 / (viewport.width * viewport.height)), 8192 / Math.max(viewport.width, viewport.height));
            canvas = document.createElement("canvas");
            canvas.width = Math.max(1, Math.floor(viewport.width * density));
            canvas.height = Math.max(1, Math.floor(viewport.height * density));
            canvas.setAttribute("role", "img");
            canvas.setAttribute("aria-label", `PDF 第 ${sheet.number} 页，共 ${pdf.numPages} 页`);
            rendering = { number: sheet.number, task: page.render({ canvasContext: canvas.getContext("2d"), viewport, transform: [density, 0, 0, density, 0, 0] }) };
            await rendering.task.promise;
            if (!stopped && current === revision && wanted.has(sheet.number)) {
              sheet.node.append(canvas);
              sheet.canvas = canvas;
              canvas = null;
            }
          } catch (error) {
            if (!stopped && current === revision && error.name !== "RenderingCancelledException") {
              sheet.failed = true;
              sheet.label.textContent = `第 ${sheet.number} 页无法渲染，可下载原文件`;
            }
          } finally {
            rendering = null;
            if (canvas) canvas.width = canvas.height = 0;
            page?.cleanup();
          }
        }
      } finally {
        pumping = false;
      }
    };
    const refresh = () => { if (!stopped) { update(); void pump(); } };
    const jump = number => {
      const target = Math.min(pdf.numPages, Math.max(1, Math.trunc(number || 1)));
      scroll.scrollTop = Math.max(0, sheets[target - 1].top - 16);
      pageInput.value = String(target);
      refresh();
    };
    const rescale = () => {
      revision++;
      rendering?.task.cancel();
      for (const sheet of sheets) { release(sheet); sheet.failed = false; sheet.label.textContent = `第 ${sheet.number} 页`; }
      layout();
      refresh();
    };
    previous.addEventListener("click", () => jump(pageNumber - 1));
    next.addEventListener("click", () => jump(pageNumber + 1));
    pageInput.addEventListener("change", () => jump(Number(pageInput.value)));
    pageInput.addEventListener("blur", () => { pageInput.value = String(pageNumber); });
    zoomOut.addEventListener("click", () => { zoom = Math.max(.5, zoom - .25); rescale(); });
    zoomIn.addEventListener("click", () => { zoom = Math.min(3, zoom + .25); rescale(); });
    scroll.addEventListener("scroll", () => {
      if (!pendingScroll) pendingScroll = window.requestAnimationFrame(() => { pendingScroll = null; refresh(); });
    }, { passive: true });
    const resize = new ResizeObserver(() => {
      clearTimeout(pendingResize);
      pendingResize = setTimeout(() => {
        if (stopped) return;
        if (width !== scroll.clientWidth) { width = scroll.clientWidth; rescale(); }
        else refresh();
      }, 150);
    });
    resize.observe(scroll);
    window.addEventListener("pagehide", event => {
      if (event.persisted) return;
      stopped = true;
      resize.disconnect(); clearTimeout(pendingResize); window.cancelAnimationFrame(pendingScroll);
      rendering?.task.cancel();
      for (const sheet of sheets) release(sheet);
      void task.destroy();
      if (downloadUrl) URL.revokeObjectURL(downloadUrl);
    });
    layout();
    scroll.scrollTop = 0;
    status.hidden = true;
    update();
    await pump();
  } catch {
    if (!status.textContent.includes("自身已加密")) status.textContent = "PDF 无法打开，可下载原文件后检查";
    if (task) { try { await task.destroy(); } catch {} }
    if (downloadUrl) window.addEventListener("pagehide", event => { if (!event.persisted) URL.revokeObjectURL(downloadUrl); });
  }
}
