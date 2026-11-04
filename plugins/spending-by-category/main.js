// Spending by category: the 30 days up to a spending table's latest day,
// each category's total as a bar, the biggest first. Data is put in the
// page as text, never as markup.
"use strict";

const DAY = 86_400_000;
const DAYS = 30;
const SHOWN = 5;
const card = document.getElementById("card");
const money = new Intl.NumberFormat("en", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const day = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", timeZone: "UTC" });

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** The currency a column's name says: euros, dollars or pounds. */
function symbolOf(name) {
  if (/eur/i.test(name)) return "€";
  if (/usd|dollar/i.test(name)) return "$";
  if (/gbp|pound/i.test(name)) return "£";
  return "";
}

/** Of these text columns, the one with the fewest different values: the likeliest category. */
function fewestValues(columns, rows) {
  let best;
  for (const column of columns) {
    const count = new Set(rows.map((row) => row[column.name])).size;
    if (count > 1 && (!best || count < best.count)) best = { column, count };
  }
  return best?.column;
}

function show(...nodes) {
  card.replaceChildren(...nodes);
}

/** A table's day, amount and category columns, if it has them. */
function columnsOf(table) {
  const date = table.columns.find((c) => c.type === "date");
  const amount = table.columns.find((c) => c.type === "number");
  const texts = table.columns.filter((c) => c.type === "text");
  const category = texts.find((c) => /categor|kind|type|group/i.test(c.name)) ?? fewestValues(texts, table.rows);
  return date && amount && category ? { date, amount, category } : undefined;
}

oikome.onData(({ collections }) => {
  if (collections.length === 0) {
    show(el("p", "empty", "Choose a table of spending for this card, in Plugins."));
    return;
  }
  // The first of its collections with a day, an amount and a category.
  const table = collections.find((c) => columnsOf(c)) ?? collections[0];
  const columns = columnsOf(table);
  if (!columns) {
    show(el("p", "empty", `${table.name} needs a day, an amount and a category for this card.`));
    return;
  }
  const { date, amount, category } = columns;
  const rows = table.rows
    .map((row) => ({ at: Date.parse(row[date.name]), value: Number(row[amount.name]), what: String(row[category.name] ?? "") }))
    .filter((row) => Number.isFinite(row.at) && Number.isFinite(row.value));
  if (rows.length === 0) {
    show(el("p", "empty", `Nothing in ${table.name} yet.`));
    return;
  }
  const end = Math.max(...rows.map((row) => row.at));
  const start = end - (DAYS - 1) * DAY;
  const totals = new Map();
  for (const row of rows) {
    if (row.at < start) continue;
    totals.set(row.what || "Other", (totals.get(row.what || "Other") ?? 0) + row.value);
  }
  const sorted = [...totals].sort((a, b) => b[1] - a[1]);
  const shown = sorted.slice(0, SHOWN);
  const rest = sorted.slice(SHOWN).reduce((sum, [, v]) => sum + v, 0);
  if (rest > 0) shown.push(["Other", rest]);
  const total = sorted.reduce((sum, [, v]) => sum + v, 0);
  const max = Math.max(...shown.map(([, v]) => v), 0);
  const symbol = symbolOf(amount.name);

  const head = el("p", "total");
  const sum = el("span");
  sum.append(el("strong", "", `${symbol}${money.format(total)}`), ` in ${DAYS} days`);
  head.append(sum, el("span", "", `${day.format(start)} – ${day.format(end)}`));

  const list = el("ol");
  list.setAttribute("aria-label", `${table.name} by category`);
  for (const [name, value] of shown) {
    const item = el("li");
    const track = el("span", "track");
    const bar = el("span", "bar");
    bar.style.width = `${max > 0 ? Math.max(2, (value / max) * 100) : 0}%`;
    track.append(bar);
    item.append(el("span", "name", name), track, el("span", "value", money.format(value)));
    list.append(item);
  }

  const open = el("button", "open", `Open ${table.name}`);
  open.type = "button";
  open.addEventListener("click", () => oikome.open(table.name));
  show(head, list, open);
});
