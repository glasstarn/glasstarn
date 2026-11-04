// Dropdowns: every single-choice <select> in the app gets a polished button
// and menu in place of the browser's own. The <select> stays in the page,
// hidden, and remains the source of truth: code reads and sets its value as
// before, and choosing from the menu fires the same change event.

let openSelectMenu = null;

// The options a menu lists, read from the <select>.
function selectMenuItems(select) {
  return [...select.options].map((option, index) => ({
    index,
    value: option.value,
    label: option.label || option.textContent || "",
    disabled: !!option.disabled,
    hidden: !!option.hidden,
    selected: index === select.selectedIndex,
  }));
}

// The next choosable item from `from` in direction `step`, wrapping around.
function nextSelectItem(items, from, step) {
  const usable = items.filter((item) => !item.disabled && !item.hidden);
  if (!usable.length) return -1;
  const order = items.map((item) => item.index);
  let position = order.indexOf(from);
  for (let tries = 0; tries < items.length; tries += 1) {
    position = (position + step + items.length) % items.length;
    const item = items[position];
    if (!item.disabled && !item.hidden) return item.index;
  }
  return usable[0].index;
}

function closeSelectMenu(focusButton = false) {
  if (!openSelectMenu) return;
  const { menu, button } = openSelectMenu;
  menu.remove();
  button.setAttribute("aria-expanded", "false");
  openSelectMenu = null;
  if (focusButton) button.focus();
}

function chooseSelectItem(select, index) {
  if (index < 0 || index === select.selectedIndex) return;
  select.selectedIndex = index;
  select.dispatchEvent(new Event("input", { bubbles: true }));
  select.dispatchEvent(new Event("change", { bubbles: true }));
}

function openSelect(select, button) {
  closeSelectMenu();
  const items = selectMenuItems(select);
  const menu = document.createElement("div");
  menu.className = "select-menu";
  menu.setAttribute("role", "listbox");
  menu.id = `${select.id || "select"}-menu`;
  let active = select.selectedIndex;
  const rows = items.map((item) => {
    const row = document.createElement("div");
    row.className = "select-option";
    row.setAttribute("role", "option");
    row.setAttribute("aria-selected", String(item.selected));
    if (item.disabled) row.setAttribute("aria-disabled", "true");
    if (item.hidden) row.hidden = true;
    row.textContent = item.label;
    row.addEventListener("mousedown", (event) => event.preventDefault());
    row.addEventListener("click", () => { if (item.disabled) return; chooseSelectItem(select, item.index); closeSelectMenu(true); });
    row.addEventListener("mousemove", () => { if (!item.disabled) highlight(item.index); });
    menu.append(row);
    return row;
  });
  const highlight = (index) => {
    active = index;
    rows.forEach((row, at) => row.classList.toggle("active", at === index));
    rows[index]?.scrollIntoView?.({ block: "nearest" });
  };
  document.body.append(menu);
  const box = button.getBoundingClientRect();
  menu.style.minWidth = `${Math.max(box.width, 160)}px`;
  const below = window.innerHeight - box.bottom;
  const height = Math.min(menu.scrollHeight, 320);
  menu.style.left = `${Math.min(box.left, window.innerWidth - menu.offsetWidth - 8)}px`;
  menu.style.top = below >= height + 12 || below > box.top ? `${box.bottom + 6}px` : `${Math.max(8, box.top - height - 6)}px`;
  button.setAttribute("aria-expanded", "true");
  button.setAttribute("aria-controls", menu.id);
  highlight(active >= 0 ? active : nextSelectItem(items, -1, 1));
  openSelectMenu = { menu, button, select, items, move: (step) => highlight(nextSelectItem(items, active, step)), choose: () => { chooseSelectItem(select, active); closeSelectMenu(true); } };
}

// Multiple choice: a searchable list of checkboxes in place of the
// browser's scroll box. The <select> stays hidden and in charge.
function multiSelectSummary(items) {
  const chosen = items.filter((item) => item.selected).length;
  return chosen ? `${chosen.toLocaleString()} selected` : "None selected";
}

function enhanceMultiSelect(select) {
  if (select.dataset.enhanced) return;
  select.dataset.enhanced = "true";
  select.classList.add("select-native");
  select.tabIndex = -1;
  select.setAttribute("aria-hidden", "true");
  const box = document.createElement("div");
  box.className = "multi-select";
  const search = document.createElement("input");
  search.type = "search";
  search.placeholder = "Find…";
  search.setAttribute("aria-label", `Find in ${select.getAttribute("aria-label") || select.closest("label")?.textContent?.trim() || "the list"}`);
  const list = document.createElement("div");
  list.className = "multi-select-list";
  list.setAttribute("role", "group");
  const foot = document.createElement("div");
  foot.className = "multi-select-foot";
  const count = document.createElement("span");
  const clear = document.createElement("button");
  clear.type = "button";
  clear.textContent = "Clear";
  foot.append(count, clear);
  box.append(search, list, foot);
  select.after(box);
  const render = () => {
    const items = selectMenuItems(select).map((item) => ({ ...item, selected: !!select.options[item.index]?.selected }));
    const term = search.value.trim().toLowerCase();
    search.hidden = items.length <= 8;
    list.replaceChildren(...items.filter((item) => !item.hidden && (!term || item.label.toLowerCase().includes(term))).map((item) => {
      const row = document.createElement("label");
      row.className = "multi-select-row";
      const check = document.createElement("input");
      check.type = "checkbox";
      check.checked = item.selected;
      check.disabled = item.disabled;
      check.addEventListener("change", () => {
        select.options[item.index].selected = check.checked;
        select.dispatchEvent(new Event("input", { bubbles: true }));
        select.dispatchEvent(new Event("change", { bubbles: true }));
        count.textContent = multiSelectSummary(selectMenuItems(select).map((other) => ({ ...other, selected: !!select.options[other.index]?.selected })));
      });
      const text = document.createElement("span");
      text.textContent = item.label;
      row.append(check, text);
      return row;
    }));
    if (!list.children.length) list.append(Object.assign(document.createElement("p"), { className: "multi-select-empty", textContent: term ? "Nothing matches." : "Nothing to choose yet." }));
    count.textContent = multiSelectSummary(items);
    clear.hidden = !items.some((item) => item.selected);
    box.hidden = select.hidden;
  };
  search.addEventListener("input", render);
  clear.addEventListener("click", () => {
    for (const option of select.options) option.selected = false;
    select.dispatchEvent(new Event("change", { bubbles: true }));
    render();
  });
  new MutationObserver(render).observe(select, { childList: true, subtree: true, attributes: true, attributeFilter: ["hidden", "disabled", "selected"] });
  // Code sets option.selected directly; redraw whenever the list comes into view.
  if (typeof ResizeObserver === "function") new ResizeObserver(() => { if (box.offsetParent) render(); }).observe(box);
  select.closest("details")?.addEventListener("toggle", render);
  render();
}

// A short choice marked data-seg becomes a sliding switch, like the map's.
function enhanceSegmentSelect(select) {
  select.dataset.enhanced = "true";
  select.classList.add("select-native");
  select.tabIndex = -1;
  select.setAttribute("aria-hidden", "true");
  const seg = document.createElement("div");
  seg.className = "seg seg-small";
  seg.setAttribute("role", "group");
  const name = select.getAttribute("aria-label") || select.closest("label")?.firstChild?.textContent?.trim();
  if (name) seg.setAttribute("aria-label", name);
  const buttons = [...select.options].map((option, index) => {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = option.label || option.textContent;
    button.addEventListener("click", () => chooseSelectItem(select, index));
    seg.append(button);
    return button;
  });
  const refresh = () => buttons.forEach((button, index) => button.setAttribute("aria-pressed", String(index === select.selectedIndex)));
  for (const property of ["value", "selectedIndex"]) {
    const native = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, property);
    Object.defineProperty(select, property, { configurable: true, get() { return native.get.call(this); }, set(value) { native.set.call(this, value); refresh(); } });
  }
  select.addEventListener("change", refresh);
  select.after(seg);
  refresh();
  if (typeof enhanceSegments === "function") enhanceSegments(seg.parentElement);
}

function enhanceSelect(select) {
  if (select.multiple) { enhanceMultiSelect(select); return; }
  if (select.dataset.seg && !select.dataset.enhanced) { enhanceSegmentSelect(select); return; }
  if (select.dataset.enhanced || select.size > 1) return;
  select.dataset.enhanced = "true";
  select.classList.add("select-native");
  select.tabIndex = -1;
  select.setAttribute("aria-hidden", "true");
  const button = document.createElement("button");
  button.type = "button";
  button.className = "select-button";
  button.setAttribute("aria-haspopup", "listbox");
  button.setAttribute("aria-expanded", "false");
  const ariaLabel = select.getAttribute("aria-label") || select.closest("label")?.firstChild?.textContent?.trim();
  if (ariaLabel) button.setAttribute("aria-label", ariaLabel);
  const text = document.createElement("span");
  text.className = "select-value";
  button.append(text);
  select.after(button);
  const refresh = () => {
    const option = select.options[select.selectedIndex];
    text.textContent = option ? option.label || option.textContent : "Choose…";
    button.disabled = select.disabled;
    button.hidden = select.hidden;
  };
  // Code that sets the value directly fires no event, so watch the setters.
  for (const property of ["value", "selectedIndex"]) {
    const native = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, property);
    Object.defineProperty(select, property, { configurable: true, get() { return native.get.call(this); }, set(value) { native.set.call(this, value); refresh(); } });
  }
  select.addEventListener("change", refresh);
  new MutationObserver(refresh).observe(select, { childList: true, subtree: true, attributes: true, attributeFilter: ["disabled", "hidden", "selected", "label"] });
  button.addEventListener("click", () => {
    if (openSelectMenu?.button === button) closeSelectMenu();
    else openSelect(select, button);
  });
  button.addEventListener("keydown", (event) => {
    const open = openSelectMenu?.button === button;
    if (["ArrowDown", "ArrowUp"].includes(event.key)) {
      event.preventDefault();
      if (!open) openSelect(select, button);
      else openSelectMenu.move(event.key === "ArrowDown" ? 1 : -1);
    } else if ((event.key === "Enter" || event.key === " ") && open) {
      event.preventDefault();
      openSelectMenu.choose();
    } else if (event.key === "Escape" && open) {
      event.preventDefault();
      closeSelectMenu(true);
    } else if (event.key === "Tab" && open) closeSelectMenu();
  });
  refresh();
}

function enhanceSelects(root = document) {
  for (const select of root.querySelectorAll?.("select") || []) enhanceSelect(select);
}

function initializeSelects() {
  enhanceSelects();
  let pending = false;
  new MutationObserver(() => {
    if (pending) return;
    pending = true;
    requestAnimationFrame(() => { pending = false; enhanceSelects(); });
  }).observe(document.body, { childList: true, subtree: true });
  document.addEventListener("mousedown", (event) => {
    if (openSelectMenu && !openSelectMenu.menu.contains(event.target) && event.target !== openSelectMenu.button && !openSelectMenu.button.contains(event.target)) closeSelectMenu();
  });
  window.addEventListener("resize", () => closeSelectMenu());
  document.addEventListener("scroll", (event) => { if (openSelectMenu && !openSelectMenu.menu.contains(event.target)) closeSelectMenu(); }, true);
}
