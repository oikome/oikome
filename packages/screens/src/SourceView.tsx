// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useEffect, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { EditorView } from "@codemirror/view";
import { languages } from "@codemirror/language-data";
import { LanguageDescription } from "@codemirror/language";
import type { LanguageSupport } from "@codemirror/language";

export default function SourceView({ text, name, wrap }: { text: string; name: string; wrap: boolean }) {
  const [language, setLanguage] = useState<LanguageSupport>();
  useEffect(() => {
    let current = true;
    setLanguage(undefined);
    const description = LanguageDescription.matchFilename(languages, name);
    void description?.load().then((support) => {
      if (current) setLanguage(support);
    });
    return () => {
      current = false;
    };
  }, [name]);

  return (
    <CodeMirror
      className="screen-source"
      value={text}
      readOnly
      editable={false}
      theme="none"
      extensions={[...(language ? [language] : []), ...(wrap ? [EditorView.lineWrapping] : [])]}
      basicSetup={{
        lineNumbers: true,
        foldGutter: true,
        highlightActiveLine: false,
        highlightActiveLineGutter: false,
      }}
    />
  );
}
