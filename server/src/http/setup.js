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

  function b64urlToBuffer(value) {
    var padded = value.replace(/-/g, "+").replace(/_/g, "/");
    while (padded.length % 4) padded += "=";
    var binary = atob(padded);
    var bytes = new Uint8Array(binary.length);
    for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes.buffer;
  }

  function bufferToB64url(buffer) {
    var bytes = new Uint8Array(buffer);
    var binary = "";
    for (var i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
    return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
  }

  function reviveCreationOptions(options) {
    options.challenge = b64urlToBuffer(options.challenge);
    options.user.id = b64urlToBuffer(options.user.id);
    if (options.excludeCredentials) {
      options.excludeCredentials = options.excludeCredentials.map(function (cred) {
        return Object.assign({}, cred, { id: b64urlToBuffer(cred.id) });
      });
    }
    return options;
  }

  function publicKeyCredentialToJSON(cred) {
    var response = cred.response;
    return {
      id: cred.id,
      rawId: bufferToB64url(cred.rawId),
      type: cred.type,
      response: {
        clientDataJSON: bufferToB64url(response.clientDataJSON),
        attestationObject: bufferToB64url(response.attestationObject),
      },
    };
  }

  var passkeyButton = document.getElementById("passkey-register");
  var passkeyStatus = document.getElementById("passkey-status");
  if (passkeyButton && window.PublicKeyCredential) {
    passkeyButton.addEventListener("click", function () {
      if (passkeyStatus) passkeyStatus.textContent = "Préparation…";
      fetch("/security/passkey/start", { method: "POST", credentials: "same-origin" })
        .then(function (response) {
          if (!response.ok) throw new Error("start failed");
          return response.json();
        })
        .then(function (body) {
          var publicKey = reviveCreationOptions(body.publicKey || body);
          return navigator.credentials.create({ publicKey: publicKey });
        })
        .then(function (cred) {
          if (!cred) throw new Error("cancelled");
          return fetch("/security/passkey/finish", {
            method: "POST",
            credentials: "same-origin",
            headers: { "content-type": "application/json" },
            body: JSON.stringify(publicKeyCredentialToJSON(cred)),
          });
        })
        .then(function (response) {
          if (!response.ok) throw new Error("finish failed");
          if (passkeyStatus) passkeyStatus.textContent = "Passkey enregistrée.";
          window.location.href = "/security";
        })
        .catch(function () {
          if (passkeyStatus) {
            passkeyStatus.textContent =
              "Enregistrement impossible. Vérifiez le navigateur et réessayez.";
          }
        });
    });
  } else if (passkeyButton && passkeyStatus) {
    passkeyStatus.textContent = "Ce navigateur ne gère pas les passkeys.";
    passkeyButton.disabled = true;
  }
})();
