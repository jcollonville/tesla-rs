(function () {
  var root = document.getElementById("debug-root");
  var frames = document.getElementById("frames");
  var logs = document.getElementById("app-logs");
  var status = document.getElementById("status");
  var pauseBtn = document.getElementById("pause");
  var live = document.getElementById("live");
  var framePane = document.getElementById("frames-scroll");
  var logPane = document.getElementById("logs-scroll");
  if (!root || !frames || !logs || !status) return;
  var paused = false;
  var queue = [];
  var stickFrames = true;
  var stickLogs = true;
  function nearBottom(el) {
    return el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  }
  if (framePane) framePane.addEventListener("scroll", function () { stickFrames = nearBottom(framePane); });
  if (logPane) logPane.addEventListener("scroll", function () { stickLogs = nearBottom(logPane); });
  function applyFilters() {
    var dir = document.getElementById("dir").value;
    var kind = document.getElementById("kind").value;
    var level = document.getElementById("level").value;
    var showKeep = document.getElementById("keepalives").checked;
    frames.classList.toggle("hide-keep", !showKeep);
    frames.querySelectorAll("li").forEach(function (li) {
      var dirOk = !dir || li.dataset.dir === dir;
      var kindOk = !kind || li.dataset.type === kind;
      li.hidden = !(dirOk && kindOk);
    });
    logs.querySelectorAll("li").forEach(function (li) {
      var value = li.dataset.level || "";
      var levelOk = !level || value === level || (level === "WARN" && value === "WARNING");
      li.hidden = !levelOk;
    });
  }
  function trim(list) {
    while (list.children.length > 400) list.removeChild(list.firstElementChild);
  }
  function appendHtml(html) {
    var tpl = document.createElement("template");
    tpl.innerHTML = html.trim();
    var li = tpl.content.firstElementChild;
    if (!li) return;
    var list = li.dataset.stream === "log" ? logs : frames;
    var sel = 'li[data-stream="' + li.dataset.stream + '"][data-seq="' + li.dataset.seq + '"]';
    if (list.querySelector(sel)) return;
    list.appendChild(li);
    trim(list);
    var empty = document.getElementById("frames-empty");
    if (empty && list === frames) empty.hidden = true;
    applyFilters();
    if (!paused) {
      if (list === frames && framePane && stickFrames) framePane.scrollTop = framePane.scrollHeight;
      if (list === logs && logPane && stickLogs) logPane.scrollTop = logPane.scrollHeight;
    }
  }
  if (pauseBtn) {
    pauseBtn.addEventListener("click", function () {
      paused = !paused;
      pauseBtn.textContent = paused ? "Resume" : "Pause";
      pauseBtn.setAttribute("aria-pressed", paused ? "true" : "false");
      if (!paused) {
        queue.forEach(appendHtml);
        queue.length = 0;
        if (framePane && stickFrames) framePane.scrollTop = framePane.scrollHeight;
        if (logPane && stickLogs) logPane.scrollTop = logPane.scrollHeight;
      }
    });
  }
  ["dir", "kind", "level", "keepalives"].forEach(function (id) {
    var el = document.getElementById(id);
    if (el) el.addEventListener("change", applyFilters);
  });
  var src = new EventSource("/logs/events?frames=" + (root.dataset.frames || "0") + "&logs=" + (root.dataset.logs || "0"));
  src.addEventListener("status", function (event) {
    if (!paused) status.innerHTML = event.data;
  });
  function onItem(event) {
    if (paused) {
      queue.push(event.data);
      if (pauseBtn) pauseBtn.textContent = "Resume (" + queue.length + ")";
      return;
    }
    appendHtml(event.data);
  }
  src.addEventListener("frame", onItem);
  src.addEventListener("log", onItem);
  src.onopen = function () { if (live) live.textContent = "live"; };
  src.onerror = function () { if (live) live.textContent = "reconnecting"; };
  applyFilters();
  if (framePane) framePane.scrollTop = framePane.scrollHeight;
  if (logPane) logPane.scrollTop = logPane.scrollHeight;
})();
