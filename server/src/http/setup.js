(function () {
  function textOf(node) {
    return node.getAttribute("data-value") || node.textContent || "";
  }

  Array.prototype.forEach.call(document.querySelectorAll("[data-toggle]"), function (button) {
    button.addEventListener("click", function () {
      var input = document.getElementById(button.getAttribute("data-toggle"));
      if (!input) return;
      var show = input.type === "password";
      input.type = show ? "text" : "password";
      button.textContent = show ? "Masquer" : "Afficher";
      button.setAttribute("aria-pressed", show ? "true" : "false");
    });
  });

  var password = document.getElementById("new-password");
  var hint = document.getElementById("password-strength");
  function strength(value) {
    if (!value) return "Saisissez un mot de passe pour voir sa solidité.";
    if (value.length < 12) return "Trop court";
    var kinds = 0;
    if (/[a-z]/.test(value)) kinds += 1;
    if (/[A-Z]/.test(value)) kinds += 1;
    if (/[0-9]/.test(value)) kinds += 1;
    if (/[^A-Za-z0-9]/.test(value)) kinds += 1;
    if (kinds >= 3 && value.length >= 16) return "Solide";
    if (kinds >= 2) return "Correct";
    return "Faible";
  }
  if (password && hint) {
    var update = function () {
      hint.textContent = strength(password.value);
    };
    password.addEventListener("input", update);
    update();
  }

  Array.prototype.forEach.call(document.querySelectorAll("[data-copy-target]"), function (button) {
    button.addEventListener("click", function () {
      var node = document.getElementById(button.getAttribute("data-copy-target"));
      if (!node || !navigator.clipboard) return;
      navigator.clipboard.writeText(textOf(node).trim());
    });
  });

  Array.prototype.forEach.call(document.querySelectorAll("[data-download]"), function (button) {
    button.addEventListener("click", function () {
      var node = document.getElementById(button.getAttribute("data-download"));
      if (!node) return;
      var blob = new Blob([textOf(node)], { type: "text/plain" });
      var url = URL.createObjectURL(blob);
      var link = document.createElement("a");
      link.href = url;
      link.download = "tesla-logger-recovery.txt";
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
    });
  });
})();
