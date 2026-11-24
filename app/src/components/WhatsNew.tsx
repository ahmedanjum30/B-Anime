import { For, Show, createResource, onMount } from "solid-js";
import type { JSX } from "solid-js";

import { whatsNew, whatsNewSeen } from "../lib/api";
import { fmtVersion } from "../lib/format";

type Block = { kind: "h"; text: string } | { kind: "li"; text: string } | { kind: "p"; text: string };

/**
 * The changelog's Markdown, as far as it goes there: "### Heading", "- item" (with indented
 * continuation lines), paragraphs, **bold** and `code`. Rendered as elements, never as HTML.
 */
function blocks(md: string): Block[] {
  const out: Block[] = [];
  for (const raw of md.split("\n")) {
    const line = raw.trimEnd();
    if (!line.trim()) continue;
    const last = out[out.length - 1];
    if (line.startsWith("### ")) out.push({ kind: "h", text: line.slice(4) });
    else if (/^\s*- /.test(line)) out.push({ kind: "li", text: line.replace(/^\s*- /, "") });
    else if (/^\s+/.test(raw) && last && last.kind !== "h") last.text += ` ${line.trim()}`;
    else if (last?.kind === "p") last.text += ` ${line.trim()}`;
    else out.push({ kind: "p", text: line.trim() });
  }
  return out;
}

function inline(text: string): JSX.Element[] {
  return text.split(/(\*\*[^*]+\*\*|`[^`]+`)/g).filter(Boolean).map((part) =>
    part.startsWith("**") ? <strong>{part.slice(2, -2)}</strong> : part.startsWith("`") ? <code>{part.slice(1, -1)}</code> : <>{part}</>,
  );
}

/** The first launch after an update: that version's changes, once. */
export default function WhatsNew() {
  const [notes] = createResource(() => whatsNew().catch(() => null));
  let dialog!: HTMLDialogElement;
  const close = () => {
    dialog.close();
    void whatsNewSeen().catch(() => {});
  };

  return (
    <Show when={notes()}>
      {(n) => {
        onMount(() => dialog.showModal());
        const list = blocks(n().notes);
        return (
          <dialog
            ref={dialog}
            class="whats-new"
            aria-labelledby="whats-new-h"
            // Esc and a click on the backdrop close it too (and count as seen).
            onCancel={(e) => {
              e.preventDefault();
              close();
            }}
            onClick={(e) => e.target === dialog && close()}
          >
            <div class="whats-new-body">
              <h2 id="whats-new-h" class="whats-new-title">
                What's new in <span class="num">{fmtVersion(n().version)}</span>
              </h2>
              <div class="whats-new-notes">
                <For each={list}>
                  {(b) =>
                    b.kind === "h" ? (
                      <h3 class="whats-new-h3">{b.text}</h3>
                    ) : b.kind === "li" ? (
                      <p class="whats-new-item">{inline(b.text)}</p>
                    ) : (
                      <p class="whats-new-p">{inline(b.text)}</p>
                    )
                  }
                </For>
              </div>
            </div>
            <div class="whats-new-foot">
              <button type="button" class="btn btn-primary" autofocus onClick={close}>
                Got it
              </button>
            </div>
          </dialog>
        );
      }}
    </Show>
  );
}
