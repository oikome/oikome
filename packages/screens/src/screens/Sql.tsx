// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Settings > Advanced > SQL Workbench: the owner's statements, for people
// who know SQL.

import { useState } from "react";
import { Button, Card, ErrorText, TextArea } from "@oikome/ui";
import type { SqlResult } from "@oikome/client";
import { useAction, useClient } from "../data";
import { RowsTable } from "../parts";
import { Link } from "../router";
import { strings } from "../strings";

const s = strings.sql;

export function Sql() {
  const client = useClient();
  const [text, setText] = useState("");
  const [result, setResult] = useState<SqlResult>();
  const run = useAction();

  return (
    <>
      <Link to="/settings">‹ {s.back}</Link>
      <header className="screen-header screen-result">
        <h1 className="screen-title">{s.title}</h1>
      </header>
      <p className="screen-lede">{s.lede}</p>
      <Card>
        <form
          onSubmit={async (e) => {
            e.preventDefault();
            setResult(undefined);
            const r = await run.run(() => client.runSql({ sql: text }));
            if (r) setResult(r);
          }}
        >
          <TextArea label={s.label} value={text} onChange={setText} />
          <Button type="submit" variant="primary" disabled={run.busy || !text.trim()}>
            {s.run}
          </Button>
        </form>
      </Card>
      {run.error && <ErrorText>{run.error}</ErrorText>}
      {result?.kind === "done" && (
        <p className="screen-status-ok screen-result" role="status">
          {s.done(result.rowsAffected)}
        </p>
      )}
      {result?.kind === "rows" && (
        <section className="screen-result">
          <p className="screen-muted" role="status">
            {s.rowCount(result.table.numRows)}
            {result.truncated ? ` · ${s.truncated}` : ""}
          </p>
          <RowsTable table={result.table} label={s.title} />
        </section>
      )}
    </>
  );
}
