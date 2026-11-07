const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { runInNewContext } = require("node:vm");

test("burned text downloads use the server filename without changing source bytes", async () => {
  for (const extension of ["md", "html", "py", "txt"]) {
    const content = "\ufeff# 中文\r\n<script>source only</script>\r\n";
    const download = { dataset: { filename: `Ab12Cd34.${extension}` } };
    const raw = {};
    const copy = { dataset: { burned: "true", raw: Buffer.from(content).toString("base64"), id: "Ab12Cd34" }, addEventListener() {} };
    let initialize;
    let blob;
    const document = {
      documentElement: { dataset: {} }, body: { addEventListener() {} },
      addEventListener(event, callback) { if (event === "DOMContentLoaded") initialize = callback; },
      querySelectorAll() { return []; },
      querySelector(selector) { return selector.includes("Download") ? download : raw; },
      getElementById(id) { return id === "copy-paste" ? copy : null; },
    };
    runInNewContext(readFileSync(`${__dirname}/../static/app.js`, "utf8"), {
      document, window: {}, localStorage: { getItem() { return null; } }, atob, Blob,
      URL: { createObjectURL(value) { blob = value; return "blob:snapshot"; } },
    });
    initialize();
    assert.equal(download.download, `Ab12Cd34.${extension}`);
    assert.equal(download.href, "blob:snapshot");
    assert.equal(raw.href, "blob:snapshot");
    assert.deepEqual(Buffer.from(await blob.arrayBuffer()), Buffer.from(content));
    assert.equal(blob.type, "text/plain;charset=utf-8");
  }
});
