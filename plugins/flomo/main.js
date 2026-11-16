// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

oikome.onData(({ collections, language }) => {
  const root = document.getElementById("notes");
  root.replaceChildren();
  const rows = collections
    .flatMap((collection) => collection.rows.map((row) => ({ row, collection: collection.name })))
    .sort((a, b) => String(b.row.created_at ?? "").localeCompare(String(a.row.created_at ?? "")))
    .slice(0, 5);
  if (rows.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = "No notes to show yet.";
    root.append(empty);
    return;
  }
  for (const { row, collection } of rows) {
    const button = document.createElement("button");
    button.type = "button";
    const text = document.createElement("span");
    text.className = "text";
    text.textContent = String(row.content ?? row.text ?? row.memo ?? "Untitled note");
    const date = document.createElement("time");
    const value = row.created_at ?? row.createdAt ?? row.date;
    if (value) {
      const parsed = new Date(String(value));
      if (!Number.isNaN(parsed.getTime())) {
        date.dateTime = parsed.toISOString();
        date.textContent = new Intl.DateTimeFormat(language, { dateStyle: "medium" }).format(parsed);
      }
    }
    button.append(text, date);
    button.addEventListener("click", () => oikome.open(collection));
    root.append(button);
  }
});
