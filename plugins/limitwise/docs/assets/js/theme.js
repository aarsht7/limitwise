(function () {
  "use strict";

  var storageKey = "limitwise-theme";
  var root = document.documentElement;
  var themeColor = document.querySelector('meta[name="theme-color"]');
  var buttons = Array.prototype.slice.call(document.querySelectorAll("[data-theme-choice]"));

  function applyTheme(theme) {
    var selectedTheme = theme === "light" ? "light" : "dark";
    root.dataset.theme = selectedTheme;
    root.style.colorScheme = selectedTheme;

    if (themeColor) {
      themeColor.content = selectedTheme === "light" ? "#e7e1d6" : "#080b12";
    }

    buttons.forEach(function (button) {
      button.setAttribute("aria-pressed", String(button.dataset.themeChoice === selectedTheme));
    });

    try {
      localStorage.setItem(storageKey, selectedTheme);
    } catch (_) {
      // The selected theme still applies when storage is unavailable.
    }
  }

  buttons.forEach(function (button) {
    button.addEventListener("click", function () {
      applyTheme(button.dataset.themeChoice);
    });
  });

  applyTheme(root.dataset.theme);
})();
