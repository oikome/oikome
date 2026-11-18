// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useEffect, useState } from "react";
import { Button, Dialog, ErrorText, Select, TextField } from "@oikome/ui";
import { useAction, useClient, useLoad } from "../data";
import { forgetPreviews } from "../content";
import { navigate } from "../router";
import { strings } from "../strings";

const s = strings.analysis;

export function DatasetAnalysisDialog({ space, table, onClose }: { space: string; table: string; onClose: () => void }) {
  const client = useClient();
  const action = useAction();
  const profile = useLoad(() => client.datasetProfile({ space: `${space}.${table}` }), [space, table]);
  const [name, setName] = useState(`${space}-analysis`);
  const [time, setTime] = useState("");
  const [amount, setAmount] = useState("");
  const [category, setCategory] = useState("");

  useEffect(() => {
    const data = profile.data;
    if (!data) return;
    const first = (role: string) => data.fields.find((field) => field.role === role)?.name ?? "";
    setTime(first("time"));
    setAmount(first("amount"));
    setCategory(first("category"));
  }, [profile.data]);

  const ready = name.trim() && time && amount && category;
  const submit = async () => {
    const result = await action.run(() =>
      client.analyzeDataset({ space, table, name: name.trim(), timeField: time, amountField: amount, categoryField: category }),
    );
    if (!result) return;
    forgetPreviews();
    onClose();
    navigate(`/collections/${encodeURIComponent(result.space.name)}`);
  };

  return (
    <Dialog
      title={s.title(`${space}.${table}`)}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button variant="primary" disabled={!ready || action.busy} onClick={() => void submit()}>
            {action.busy ? s.creating : s.action}
          </Button>
        </>
      }
    >
      <p className="oik-dialog__text">{s.body}</p>
      <TextField name="analysis-name" label={s.name} value={name} mono onChange={setName} />
      {(["time", "amount", "category"] as const).map((role) => {
        const value = role === "time" ? time : role === "amount" ? amount : category;
        const setValue = role === "time" ? setTime : role === "amount" ? setAmount : setCategory;
        return (
          <Select
            key={role}
            label={s.fields[role]}
            value={value}
            onChange={setValue}
            options={(profile.data?.fields ?? []).map((field) => ({
              value: field.name,
              label: `${field.name} · ${field.dataType}`,
            }))}
          />
        );
      })}
      {profile.error && <ErrorText>{profile.error}</ErrorText>}
      {action.error && <ErrorText>{action.error}</ErrorText>}
    </Dialog>
  );
}
