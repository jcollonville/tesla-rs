(function () {
  var lists = document.querySelectorAll('[role="tablist"]');
  Array.prototype.forEach.call(lists, function (list) {
    var tabs = Array.prototype.filter.call(
      list.querySelectorAll('[role="tab"]'),
      function (tab) {
        return tab.getAttribute("href");
      }
    );
    tabs.forEach(function (tab, index) {
      tab.addEventListener("keydown", function (event) {
        var next = null;
        if (event.key === "ArrowRight") next = tabs[(index + 1) % tabs.length];
        if (event.key === "ArrowLeft") next = tabs[(index - 1 + tabs.length) % tabs.length];
        if (event.key === "Home") next = tabs[0];
        if (event.key === "End") next = tabs[tabs.length - 1];
        if (!next) return;
        event.preventDefault();
        next.focus();
      });
    });
  });
})();
