// Photo memories: of the photos it may read, the one taken nearest to
// this day of the year in an earlier year, as a card. Data is put in the
// page as text and image addresses, never as markup.
"use strict";

const DAY = 86_400_000;
/** Within this many days of today's date it is "a year ago"; further, the day it was taken. */
const NEAR = 15;
const card = document.getElementById("card");
const long = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" });

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

oikome.onData(({ collections, today }) => {
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
    card.replaceChildren(el("p", "empty", "Choose photos for this card, in Plugins."));
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
      ? years === 1
        ? "One year ago"
        : `${years} years ago`
      : `From ${pick.collection}`;

  const button = el("button", "photo");
  button.type = "button";
  button.setAttribute("aria-label", `${pick.title || "A photo"}, ${long.format(pick.at)}. Open ${pick.collection}`);
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
