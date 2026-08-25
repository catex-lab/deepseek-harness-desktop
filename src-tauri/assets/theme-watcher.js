(function () {
  "use strict";

  /* Theme watcher v4-diag.
     Every lifecycle stage beacons home unconditionally so the shell log
     shows exactly where the chain lives or dies. Install-once guard is
     per-version, so upgrades replace older watchers cleanly. */

  var V = "v4";

  function b(sig) {
    try {
      fetch(
        "http://127.0.0.1:38080/beacon?dark=0&sig=" +
          encodeURIComponent("diag:" + V + ":" + sig),
        { mode: "no-cors", cache: "no-store" }
      ).catch(function () {});
    } catch (e) {}
  }

  b("entry loc=" + String(location.href).slice(0, 60));

  /* Replaced wholesale on each upgrade. */
  var GUARD = "__dshThemeWatchV4";
  try {
    if (window[GUARD]) return; /* silent - self-heal reinjects are routine */
    window[GUARD] = true;
  } catch (e) {}

  function lum(rgbStr) {
    var m = /rgba?\(([^)]+)\)/.exec(rgbStr || "");
    if (!m) return null;
    var p = m[1].split(",").map(parseFloat);
    if (p.length < 3 || isNaN(p[0]) || isNaN(p[1]) || isNaN(p[2])) return null;
    var a = p.length > 3 ? p[3] : 1;
    if (a === 0) return null;
    function f(c) { c /= 255; return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4); }
    return 0.2126 * f(p[0]) + 0.7152 * f(p[1]) + 0.0722 * f(p[2]);
  }

  function explicitSignal() {
    try {
      var d = document.documentElement;
      var s = d.getAttribute("style") || "";
      var m = s.match(/color-scheme\s*:\s*([a-z-]+)/i);
      if (m && /dark|light/i.test(m[1]))
        return { dark: /dark/i.test(m[1]), src: "html.color-scheme:" + m[1] };
      var keys = ["data-theme", "data-color-mode", "data-bs-theme"];
      for (var i = 0; i < keys.length; i++) {
        var v = d.getAttribute(keys[i]);
        if (v) return { dark: /dark/i.test(v) && !/light/i.test(v), src: keys[i] + "=" + v };
        if (document.body) {
          var bv = document.body.getAttribute(keys[i]);
          if (bv !== null && bv !== "")
            return { dark: /dark/i.test(bv) && !/light/i.test(bv), src: "body." + keys[i] + "=" + bv };
        }
      }
      if (/(^|\s)dark(\s|$)/.test(d.className || ""))
        return { dark: true, src: "html.class.dark" };
      if (document.body && /(^|\s)dark(\s|$)/.test(document.body.className || ""))
        return { dark: true, src: "body.class.dark" };
    } catch (e) {}
    return null;
  }

  function lsProbe() {
    try {
      var ks = ["theme", "color-scheme", "colorScheme", "ds-theme", "dsh-theme",
                "darkMode", "dark", "mode", "appearance"];
      for (var i = 0; i < ks.length; i++) {
        var v = localStorage.getItem(ks[i]);
        if (v !== null && v !== "")
          return {
            dark: /dark/i.test(v) && !/light/i.test(v),
            src: "ls." + ks[i] + "=" + String(v).slice(0, 40),
          };
      }
    } catch (e) {}
    return null;
  }

  function derivedSignal() {
    try {
      var els = [document.documentElement];
      if (document.body) els.push(document.body);
      var extra = document.querySelectorAll("body *");
      var n = Math.min(extra.length, 300);
      for (var j = 0; j < n; j++) els.push(extra[j]);
      for (var k = 0; k < els.length; k++) {
        var L = lum(getComputedStyle(els[k]).backgroundColor);
        if (L !== null)
          return {
            dark: L < 0.25,
            src: "deep[" + k + "]<" + (els[k].tagName || "?") + ">.lum=" + L.toFixed(2),
          };
      }
    } catch (e) {}
    return null;
  }

  function detect() {
    return explicitSignal() || lsProbe() || derivedSignal();
  }

  var last = null;
  var sentReal = false;

  function report(r) {
    var delivered = false;
    try {
      var t = window.__TAURI__ && window.__TAURI__.core;
      if (t && t.invoke) {
        t.invoke("webui_theme_changed", { dark: r.dark, sig: String(r.src).slice(0, 120) });
        delivered = true;
      }
    } catch (e) {}
    if (!delivered) b("fallback-real dark=" + (r.dark ? 1 : 0) + " src=" + String(r.src).slice(0, 60));
  }

  function tick() {
    var r = detect();
    if (!r) return;
    if (sentReal && r.dark === last) return;
    last = r.dark;
    sentReal = true;
    report(r);
  }

  /* Observers - deferred until a usable DOM exists (early evals can
     land on an empty document where documentElement is null). */
  var htmlObs = null;
  function attachHtml() {
    if (htmlObs || !document.documentElement) return;
    try {
      htmlObs = new MutationObserver(tick);
      htmlObs.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["style", "class", "data-theme", "data-color-mode", "data-bs-theme"],
      });
      b("observers-html-attached");
    } catch (e) {
      htmlObs = null;
      b("observers-html-ERR " + e);
    }
  }
  attachHtml();

  function watchBody() {
    try {
      if (!document.body) return false;
      new MutationObserver(tick).observe(document.body, {
        attributes: true,
        attributeFilter: ["style", "class", "data-theme"],
      });
      return true;
    } catch (e) { b("observers-body-ERR " + e); return false; }
  }

  try {
    var mq = window.matchMedia("(prefers-color-scheme: dark)");
    var onMq = function () { tick(); };
    if (mq.addEventListener) mq.addEventListener("change", onMq);
    else if (mq.addListener) mq.addListener(onMq);
  } catch (e) {}

  /* Safety poller - permanent, cheap (dedup makes repeats free). */
  setInterval(tick, 1000);

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      watchBody();
      attachHtml();
      b("domcontentloaded");
      tick();
    });
  } else {
    watchBody();
    b("doc-ready");
  }

  tick();

  /* Heartbeat: every 15s while we have never delivered a REAL signal.
     Proves liveness and pinpoints blindness vs death. */
  var beats = 0;
  setInterval(function () {
    if (!sentReal && beats < 6) {
      beats++;
      b("heartbeat-" + beats + " loc=" + String(location.href).slice(0, 40));
    }
  }, 15000);
})();
