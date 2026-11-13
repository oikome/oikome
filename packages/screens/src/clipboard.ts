// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import type { Table } from "apache-arrow";
import { columnLabel } from "./format";
import { show } from "./parts";

export async function writeClipboard(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch (error) {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    const copied = document.execCommand("copy");
    area.remove();
    if (!copied) throw error;
  }
}

function cell(field: Table["schema"]["fields"][number], value: unknown): string {
  const text = show(field, value) ?? "";
  return /["\t\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

export function tableText(table: Table): string {
  const fields = table.schema.fields;
  const lines = [fields.map((field) => cell(field, columnLabel(field.name))).join("\t")];
  for (let row = 0; row < table.numRows; row++) {
    lines.push(fields.map((field, column) => cell(field, table.getChildAt(column)?.get(row) as unknown)).join("\t"));
  }
  return lines.join("\n");
}
