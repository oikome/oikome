// Photo memories: of the photos it may read, the one taken nearest to
// this day of the year in an earlier year, as a card. Data is put in the
// page as text and image addresses, never as markup.
"use strict";

const DAY = 86_400_000;
/** Within this many days of today's date it is "a year ago"; further, the day it was taken. */
const NEAR = 15;
const card = document.getElementById("card");

/** A Russian noun's form for `n`: 1 год, 2 года, 5 лет. */
function russian(n, one, few, many) {
  const form = new Intl.PluralRules("ru").select(n);
  return form === "one" ? one : form === "few" ? few : many;
}

/** The card's words in each language the app is written in; British English reads the American. */
const WORDS = {
  en: {
    empty: "Choose photos for this card, in Plugins.",
    yearsAgo: (n) => (n === 1 ? "One year ago" : `${n} years ago`),
    from: (c) => `From ${c}`,
    aPhoto: "A photo",
    open: (title, date, c) => `${title}, ${date}. Open ${c}`,
  },
  de: {
    empty: "Wähle unter Plugins Fotos für diese Karte.",
    yearsAgo: (n) => (n === 1 ? "Vor einem Jahr" : `Vor ${n} Jahren`),
    from: (c) => `Aus ${c}`,
    aPhoto: "Ein Foto",
    open: (title, date, c) => `${title}, ${date}. ${c} öffnen`,
  },
  fr: {
    empty: "Choisissez des photos pour cette carte, dans Plugins.",
    yearsAgo: (n) => (n === 1 ? "Il y a un an" : `Il y a ${n} ans`),
    from: (c) => `De ${c}`,
    aPhoto: "Une photo",
    open: (title, date, c) => `${title}, ${date}. Ouvrir ${c}`,
  },
  ja: {
    empty: "このカードに使う写真を、プラグインで選んでください。",
    yearsAgo: (n) => `${n} 年前`,
    from: (c) => `${c} から`,
    aPhoto: "写真",
    open: (title, date, c) => `${title}、${date}。${c} を開く`,
  },
  ko: {
    empty: "플러그인에서 이 카드에 쓸 사진을 고르세요.",
    yearsAgo: (n) => `${n}년 전`,
    from: (c) => `${c}에서`,
    aPhoto: "사진",
    open: (title, date, c) => `${title}, ${date}. ${c} 열기`,
  },
  ru: {
    empty: "Выберите фото для этой карточки в разделе «Плагины».",
    yearsAgo: (n) => (n === 1 ? "Год назад" : `${n} ${russian(n, "год", "года", "лет")} назад`),
    from: (c) => `Из ${c}`,
    aPhoto: "Фото",
    open: (title, date, c) => `${title}, ${date}. Открыть ${c}`,
  },
  "zh-Hans": {
    empty: "在“插件”里为这张卡片选择照片。",
    yearsAgo: (n) => `${n} 年前`,
    from: (c) => `来自 ${c}`,
    aPhoto: "一张照片",
    open: (title, date, c) => `${title}，${date}。打开 ${c}`,
  },
  "zh-Hant": {
    empty: "在「外掛程式」裡為這張卡片選擇照片。",
    yearsAgo: (n) => `${n} 年前`,
    from: (c) => `來自 ${c}`,
    aPhoto: "一張照片",
    open: (title, date, c) => `${title}，${date}。開啟 ${c}`,
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

/** Days between two dates' places in the year, either way round. */
function daysApart(a, b) {
  const yearDay = (d) => (Date.UTC(2001, d.getUTCMonth(), d.getUTCDate()) - Date.UTC(2001, 0, 1)) / DAY;
  const gap = Math.abs(yearDay(a) - yearDay(b));
  return Math.min(gap, 365 - gap);
}

oikome.onData(({ collections, today, language }) => {
  const w = wordsFor(language);
  const long = new Intl.DateTimeFormat(language, { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" });
  const now = new Date(`${today}T00:00:00Z`);
  const photos = [];
  for (const collection of collections) {
    const image = collection.columns.find((c) => c.type === "image");
    const date = collection.columns.find((c) => c.type === "date");
    if (!image || !date) continue;
    const texts = collection.columns.filter((c) => c.type === "text");
    const title = texts.find((c) => /title|name|caption/i.test(c.name)) ?? texts[0];
    const place = texts.find((c) => /place|where|location|city/i.test(c.name));
    for (const row of collection.rows) {
      const at = new Date(String(row[date.name]).slice(0, 10) + "T00:00:00Z");
      if (typeof row[image.name] !== "string" || Number.isNaN(at.getTime())) continue;
      photos.push({
        collection: collection.name,
        src: row[image.name],
        at,
        title: title ? String(row[title.name] ?? "") : "",
        place: place ? String(row[place.name] ?? "") : "",
      });
    }
  }
  if (photos.length === 0) {
    card.replaceChildren(el("p", "empty", w.empty));
    return;
  }
  // An earlier year's photo nearest this day; else the latest one.
  const earlier = photos.filter((p) => p.at.getUTCFullYear() < now.getUTCFullYear());
  const pick = earlier.length
    ? earlier.reduce((best, p) => {
        const d = daysApart(p.at, now) - daysApart(best.at, now);
        return d < 0 || (d === 0 && p.at > best.at) ? p : best;
      })
    : photos.reduce((best, p) => (p.at > best.at ? p : best));
  const years = now.getUTCFullYear() - pick.at.getUTCFullYear();
  const label =
    earlier.length && daysApart(pick.at, now) <= NEAR
      ? w.yearsAgo(years)
      : w.from(pick.collection);

  const button = el("button", "photo");
  button.type = "button";
  button.setAttribute("aria-label", w.open(pick.title || w.aPhoto, long.format(pick.at), pick.collection));
  const img = el("img");
  img.alt = "";
  img.src = pick.src;
  const plate = el("span", "plate");
  plate.append(
    el("span", "label", label),
    el("span", "title", pick.title || pick.collection),
    el("span", "meta", [long.format(pick.at), pick.place].filter(Boolean).join(" · ")),
  );
  button.append(img, plate);
  button.addEventListener("click", () => oikome.open(pick.collection));
  card.replaceChildren(button);
});
