/*
 * Progressive enhancement, and nothing else.
 *
 * The form works with JavaScript switched off — it just offers a fixed number
 * of blank rows instead of growing on demand. Everything here is about making
 * that nicer, never about making it work. If this file fails to load, nobody
 * is locked out of the waiting list.
 */
(function () {
  "use strict";

  /** Renumber a row so its ids, labels and checkbox value match its position. */
  function renumber(row, index) {
    row.querySelectorAll("input, select, textarea").forEach(function (field) {
      if (field.id) {
        var base = field.id.replace(/_\d+$/, "");
        var label = row.querySelector('label[for="' + field.id + '"]');
        field.id = base + "_" + index;
        if (label) label.setAttribute("for", field.id);
      }
      // The "tell them" checkbox carries its row index as its value; that is
      // what keeps the columns lined up on the server when only some are
      // ticked. Getting this wrong would tell the wrong person.
      if (field.name === "contact_tell") field.value = String(index);
    });
  }

  function addRow(container) {
    var rows = container.querySelectorAll(".row");
    var last = rows[rows.length - 1];
    if (!last) return;

    var copy = last.cloneNode(true);

    copy.querySelectorAll("input, select, textarea").forEach(function (field) {
      if (field.type === "checkbox" || field.type === "radio") {
        field.checked = false;
      } else if (field.tagName === "SELECT") {
        field.selectedIndex = 0;
      } else {
        field.value = "";
      }
    });

    container.appendChild(copy);
    Array.prototype.forEach.call(container.querySelectorAll(".row"), renumber);

    var first = copy.querySelector("input, select, textarea");
    if (first) first.focus();
  }

  document.querySelectorAll("[data-add-row]").forEach(function (button) {
    var container = document.getElementById(button.getAttribute("data-add-row"));
    if (!container) return;

    // Hidden in the markup so it never appears without a handler behind it.
    button.hidden = false;
    button.addEventListener("click", function () {
      addRow(container);
    });
  });

  // A bounced submission should land you on the reason, not at the top of a
  // long page wondering what went wrong.
  var alert = document.getElementById("form-error");
  if (alert) {
    alert.focus();
    alert.scrollIntoView({ block: "center", behavior: "smooth" });
  }
})();
