// Spending by category: the 30 days up to a spending table's latest day,
// each category's total as a bar, the biggest first. Data is put in the
// page as text, never as markup.
"use strict";

const DAY = 86_400_000;
const DAYS = 30;
const SHOWN = 5;
const card = document.getElementById("card");

/** A Russian noun's form for `n`: 1 день, 2 дня, 5 дней. */
function russian(n, one, few, many) {
  const form = new Intl.PluralRules("ru").select(n);
  return form === "one" ? one : form === "few" ? few : many;
}

/** The card's words in each language the app is written in; British English reads the American. */
const WORDS = {
  en: {
    empty: "Choose a table of spending for this card, in Plugins.",
    needs: (t) => `${t} needs a day, an amount and a category for this card.`,
    nothing: (t) => `Nothing in ${t} yet.`,
    other: "Other",
    total: (n) => ["", ` in ${n} days`],
    byCategory: (t) => `${t} by category`,
    open: (t) => `Open ${t}`,
  },
  de: {
    empty: "Wähle unter Plugins eine Ausgabentabelle für diese Karte.",
    needs: (t) => `${t} braucht für diese Karte einen Tag, einen Betrag und eine Kategorie.`,
    nothing: (t) => `In ${t} ist noch nichts.`,
    other: "Sonstiges",
    total: (n) => ["", ` in ${n} Tagen`],
    byCategory: (t) => `${t} nach Kategorie`,
    open: (t) => `${t} öffnen`,
  },
  fr: {
    empty: "Choisissez une table de dépenses pour cette carte, dans Plugins.",
    needs: (t) => `${t} doit avoir un jour, un montant et une catégorie pour cette carte.`,
    nothing: (t) => `Rien dans ${t} pour l’instant.`,
    other: "Autres",
    total: (n) => ["", ` sur ${n} jours`],
    byCategory: (t) => `${t} par catégorie`,
    open: (t) => `Ouvrir ${t}`,
  },
  ja: {
    empty: "このカードに使う支出の表を、プラグインで選んでください。",
    needs: (t) => `このカードには、${t} に日付・金額・カテゴリが必要です。`,
    nothing: (t) => `${t} にはまだ何もありません。`,
    other: "その他",
    total: (n) => [`${n} 日間で `, ""],
    byCategory: (t) => `${t}（カテゴリ別）`,
    open: (t) => `${t} を開く`,
  },
  ko: {
    empty: "플러그인에서 이 카드에 쓸 지출 표를 고르세요.",
    needs: (t) => `이 카드에는 ${t}에 날짜, 금액, 카테고리가 있어야 합니다.`,
    nothing: (t) => `${t}에 아직 아무것도 없습니다.`,
    other: "기타",
    total: (n) => [`${n}일 동안 `, ""],
    byCategory: (t) => `카테고리별 ${t}`,
    open: (t) => `${t} 열기`,
  },
  ru: {
    empty: "Выберите таблицу расходов для этой карточки в разделе «Плагины».",
    needs: (t) => `Для этой карточки в ${t} нужны день, сумма и категория.`,
    nothing: (t) => `В ${t} пока ничего нет.`,
    other: "Прочее",
    total: (n) => ["", ` за ${n} ${russian(n, "день", "дня", "дней")}`],
    byCategory: (t) => `${t} по категориям`,
    open: (t) => `Открыть ${t}`,
  },
  "zh-Hans": {
    empty: "在“插件”里为这张卡片选一张消费表。",
    needs: (t) => `这张卡片需要 ${t} 有日期、金额和类别。`,
    nothing: (t) => `${t} 里还没有内容。`,
    other: "其他",
    total: (n) => [`${n} 天共 `, ""],
    byCategory: (t) => `${t}（按类别）`,
    open: (t) => `打开 ${t}`,
  },
  "zh-Hant": {
    empty: "在「外掛程式」裡為這張卡片選一張消費表。",
    needs: (t) => `這張卡片需要 ${t} 有日期、金額和類別。`,
    nothing: (t) => `${t} 裡還沒有內容。`,
    other: "其他",
    total: (n) => [`${n} 天共 `, ""],
    byCategory: (t) => `${t}（依類別）`,
    open: (t) => `開啟 ${t}`,
  },
};

/** The words for `language`: its own, else its base language's, else English. */
function wordsFor(language) {
  return WORDS[language] ?? WORDS[language.split("-")[0]] ?? WORDS.en;
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** The currency a column's name says: euros, dollars or pounds. */
function currencyOf(name) {
  if (/eur|欧元|歐元|ユーロ|유로|евро/i.test(name)) return "EUR";
  if (/usd|dollar/i.test(name)) return "USD";
  if (/gbp|pound/i.test(name)) return "GBP";
  return undefined;
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

oikome.onData(({ collections, language }) => {
  const w = wordsFor(language);
  const day = new Intl.DateTimeFormat(language, { month: "short", day: "numeric", timeZone: "UTC" });
  if (collections.length === 0) {
    show(el("p", "empty", w.empty));
    return;
  }
  // The first of its collections with a day, an amount and a category.
  const table = collections.find((c) => columnsOf(c)) ?? collections[0];
  const columns = columnsOf(table);
  if (!columns) {
    show(el("p", "empty", w.needs(table.name)));
    return;
  }
  const { date, amount, category } = columns;
  const rows = table.rows
    .map((row) => ({ at: Date.parse(row[date.name]), value: Number(row[amount.name]), what: String(row[category.name] ?? "") }))
    .filter((row) => Number.isFinite(row.at) && Number.isFinite(row.value));
  if (rows.length === 0) {
    show(el("p", "empty", w.nothing(table.name)));
    return;
  }
  const end = Math.max(...rows.map((row) => row.at));
  const start = end - (DAYS - 1) * DAY;
  const totals = new Map();
  for (const row of rows) {
    if (row.at < start) continue;
    totals.set(row.what || w.other, (totals.get(row.what || w.other) ?? 0) + row.value);
  }
  const sorted = [...totals].sort((a, b) => b[1] - a[1]);
  const shown = sorted.slice(0, SHOWN);
  const rest = sorted.slice(SHOWN).reduce((sum, [, v]) => sum + v, 0);
  if (rest > 0) shown.push([w.other, rest]);
  const total = sorted.reduce((sum, [, v]) => sum + v, 0);
  const max = Math.max(...shown.map(([, v]) => v), 0);
  const currency = currencyOf(amount.name);
  const digits = { minimumFractionDigits: 2, maximumFractionDigits: 2 };
  const money = new Intl.NumberFormat(language, digits);
  const priced = currency ? new Intl.NumberFormat(language, { style: "currency", currency, ...digits }) : money;

  const head = el("p", "total");
  const sum = el("span");
  const [before, after] = w.total(DAYS);
  sum.append(before, el("strong", "", priced.format(total)), after);
  head.append(sum, el("span", "", day.formatRange(start, end)));

  const list = el("ol");
  list.setAttribute("aria-label", w.byCategory(table.name));
  for (const [name, value] of shown) {
    const item = el("li");
    const track = el("span", "track");
    const bar = el("span", "bar");
    bar.style.width = `${max > 0 ? Math.max(2, (value / max) * 100) : 0}%`;
    track.append(bar);
    item.append(el("span", "name", name), track, el("span", "value", money.format(value)));
    list.append(item);
  }

  const open = el("button", "open", w.open(table.name));
  open.type = "button";
  open.addEventListener("click", () => oikome.open(table.name));
  show(head, list, open);
});
