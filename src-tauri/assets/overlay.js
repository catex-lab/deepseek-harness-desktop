(function () {
  "use strict";

  var INJECTED_TAG = "data-dsh-desktop-injected";
  var BAR_ID = "dsh-desktop-titlebar";

  /* ---- IPC helpers ------------------------------------------------------ */
  function invoke(cmd, args) {
    try {
      if (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) {
        return window.__TAURI__.core.invoke(cmd, args || {}).catch(function (e) {
          report("invoke-error", cmd + "|" + (e.message || e));
        });
      }
      return Promise.resolve();
    } catch (e) {
      return Promise.resolve();
    }
  }

  function report(event, payload) {
    try {
      invoke("log_engine_event", { event: event, payload: payload || "" });
    } catch (e) {}
    try {
      if (console && console.log) console.log("[dsh-desktop] " + event + " | " + (payload || ""));
    } catch (e) {}
  }

  /* ---- Guard: inject once ----------------------------------------------- */
  if (document.documentElement.getAttribute(INJECTED_TAG) === "true") return;
  document.documentElement.setAttribute(INJECTED_TAG, "true");

  /* ---- Theme sync ------------------------------------------------------- */
  function isDarkHtmlStyle(s) {
    if (!s) return false;
    return s.indexOf("color-scheme") !== -1 && s.indexOf("dark") !== -1;
  }

  function applyTheme() {
    var cs = document.documentElement.getAttribute("style") || "";
    var dark = isDarkHtmlStyle(cs);
    invoke("set_theme", { theme: dark ? "dark" : "light" });
    var bar = document.getElementById(BAR_ID);
    if (bar) {
      bar.setAttribute("data-dsh-theme", dark ? "dark" : "light");
    }
    report("theme", (dark ? "dark" : "light") + " | htmlStyle=" + cs.substring(0, 120));
  }

  /* ---- Title sync ------------------------------------------------------- */
  function updateTitle() {
    var t = document.title || "DeepSeek Harness";
    var bar = document.getElementById(BAR_ID);
    if (bar) {
      var el = bar.querySelector(".dsh-title-text");
      if (el) el.textContent = t;
    }
  }

  /* ---- Bar construction ------------------------------------------------- */
  function createBar() {
    var bar = document.createElement("div");
    bar.id = BAR_ID;
    bar.setAttribute("data-dsh-desktop-titlebar", "true");
    bar.draggable = false;
    bar.style.userSelect = "none";
    bar.style.touchAction = "none";

    var logoSvg =
      "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 1024 1024'>" +
      "<circle cx='512' cy='512' r='512' fill='%231D1D1F'/>" +
      "<text x='512' y='612' font-size='560' font-family='Arial' font-weight='bold' fill='%23FFFFFF' text-anchor='middle'>D</text>" +
      "</svg>";

    bar.innerHTML =
      '<div class="dsh-brand"><img class="dsh-brand-logo" src="data:image/svg+xml;utf8,' +
      logoSvg +
      '" alt="D"><span class="dsh-title-text">' +
      (document.title || "DeepSeek Harness") +
      '</span><span class="dsh-sep">·</span><span class="dsh-mode">DeepSeek Harness</span></div>' +
      '<div class="dsh-controls">' +
      '  <button class="dsh-btn" id="dsh-minimize"  title="Minimize">&#x2015;</button>' +
      '  <button class="dsh-btn" id="dsh-maximize"  title="Maximize">&#x25A1;</button>' +
      '  <button class="dsh-btn dsh-close" id="dsh-close" title="Close">&#x2715;</button>' +
      "</div>";

    /* Drag: Tauri v2 window.startDragging() via __TAURI__ api; fall back to no-op */
    bar.addEventListener("mousedown", function (e) {
      if (e.button !== 0) return;
      /* Don't drag when clicking a button */
      if (e.target && e.target.closest && e.target.closest(".dsh-btn")) return;
      e.stopPropagation();
      try {
        if (
          window.__TAURI__ &&
          window.__TAURI__.core &&
          window.__TAURI__.core.startDragging
        ) {
          window.__TAURI__.core.startDragging();
        }
      } catch (err) {
        report("drag-error", err.message || String(err));
      }
    });

    /* Buttons */
    var minimize = bar.querySelector("#dsh-minimize");
    var maximize = bar.querySelector("#dsh-maximize");
    var closeBtn = bar.querySelector("#dsh-close");

    if (minimize)
      minimize.addEventListener("click", function () {
        invoke("minimize_window");
      });
    if (maximize)
      maximize.addEventListener("click", function () {
        invoke("toggle_maximize_window");
      });
    if (closeBtn)
      closeBtn.addEventListener("click", function () {
        invoke("close_window");
      });

    /* Double-click to maximize (excluding buttons) */
    bar.addEventListener("dblclick", function (e) {
      if (e.target && e.target.closest && e.target.closest(".dsh-btn")) return;
      invoke("toggle_maximize_window");
    });

    /* Inline styles: light theme default, dark via [data-dsh-theme="dark"] */
    var style = document.createElement("style");
    style.textContent = [
      "#dsh-desktop-titlebar{",
      "  position:fixed;top:0;left:0;right:0;height:42px;",
      "  display:flex;align-items:center;padding:0 12px 0 16px;",
      "  box-sizing:border-box;",
      "  z-index:2147483647;",
      "  color:#1d1d1f;",
      "  font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;",
      "}",
      "#dsh-desktop-titlebar .dsh-brand{display:flex;align-items:center;gap:10px;min-width:0;flex:1}",
      "#dsh-desktop-titlebar .dsh-brand-logo{width:18px;height:18px;display:block;flex-shrink:0}",
      "#dsh-desktop-titlebar .dsh-title-text{font-size:13px;font-weight:500;line-height:1.5;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}",
      "#dsh-desktop-titlebar .dsh-sep{margin:0 6px;color:#1d1d1f26;font-size:13px;flex-shrink:0}",
      "#dsh-desktop-titlebar .dsh-mode{font-size:12px;color:#1d1d1f99;letter-spacing:0.3px;flex-shrink:0}",
      "#dsh-desktop-titlebar .dsh-controls{display:flex;align-items:center;gap:0;margin-left:12px}",
      "#dsh-desktop-titlebar .dsh-btn{",
      "  display:inline-flex;align-items:center;justify-content:center;",
      "  width:42px;height:42px;border:none;background:transparent;color:#1d1d1f;",
      "  font-size:14px;cursor:pointer;padding:0;outline:none;line-height:1;",
      "}",
      "#dsh-desktop-titlebar .dsh-btn:hover{background-color:rgba(0,0,0,0.06);color:#1d1d1f}",
      "#dsh-desktop-titlebar .dsh-close:hover{background-color:#e81123;color:#fff}",
      "#dsh-desktop-titlebar[data-dsh-theme='dark']{color:#e6e6ea}",
      "#dsh-desktop-titlebar[data-dsh-theme='dark'] .dsh-sep{color:#e6e6ea33}",
      "#dsh-desktop-titlebar[data-dsh-theme='dark'] .dsh-mode{color:#e6e6ea80}",
      "#dsh-desktop-titlebar[data-dsh-theme='dark'] .dsh-btn{color:#e6e6ea}",
      "#dsh-desktop-titlebar[data-dsh-theme='dark'] .dsh-btn:hover{background-color:rgba(255,255,255,0.08);color:#e6e6ea}",
      "#dsh-desktop-titlebar[data-dsh-theme='dark'] .dsh-close:hover{background-color:#e81123;color:#fff}",
      "body{padding-top:42px !important;box-sizing:border-box !important;margin:0}",
    ].join("\n");
    bar.appendChild(style);
    return bar;
  }

  function inject() {
    var existing = document.getElementById(BAR_ID);
    if (existing) existing.remove();
    var bar = createBar();
    document.body.appendChild(bar);

    applyTheme();

    /* Observe theme changes on html.style */
    var htmlObs = new MutationObserver(function () {
      applyTheme();
    });
    htmlObs.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["style"],
    });

    /* Observe title changes */
    var titleObs = new MutationObserver(updateTitle);
    var head = document.head;
    if (head) {
      titleObs.observe(head, { childList: true, subtree: true });
      var titleEl = head.querySelector("title");
      if (titleEl) {
        titleObs.observe(titleEl, {
          childList: true,
          characterData: true,
        });
      }
    }

    var cs = document.documentElement.getAttribute("style") || "";
    report(
      "titlebar-injected",
      "theme=" +
        (isDarkHtmlStyle(cs) ? "dark" : "light") +
        " | htmlStyle=" +
        cs.substring(0, 120)
    );

    /* Re-apply theme a tick later to catch late-init dark-mode pages */
    setTimeout(applyTheme, 250);
  }

  /* Wait for DOM so body exists */
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", inject);
  } else {
    inject();
  }
})();