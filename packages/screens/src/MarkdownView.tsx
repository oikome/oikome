// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

export default function MarkdownView({ text }: { text: string }) {
  return (
    <article className="screen-markdown">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        skipHtml
        components={{
          a: ({ children, href, ...props }) =>
            href ? (
              <a {...props} href={href} target="_blank" rel="noreferrer nofollow">
                {children}
              </a>
            ) : (
              <span>{children}</span>
            ),
          img: ({ alt }) => <span className="screen-markdown__image-alt">{alt}</span>,
          input: ({ checked, type, ...props }) =>
            type === "checkbox" ? <input {...props} type="checkbox" checked={checked} disabled readOnly /> : null,
        }}
      >
        {text}
      </ReactMarkdown>
    </article>
  );
}
