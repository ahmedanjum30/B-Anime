import type { Mark as MarkKind } from "../lib/types";

const LABEL: Record<MarkKind, { text: string; desc: string }> = {
  new: { text: "NEW", desc: "New episode" },
  final: { text: "FINAL", desc: "Final episode" },
};

/** Boxed listings mark. At most one per item (the backend guarantees it). */
export default function Mark(props: { kind: MarkKind }) {
  return (
    <span class={`mark mark-${props.kind}`} title={LABEL[props.kind].desc}>
      <span aria-hidden="true">{LABEL[props.kind].text}</span>
      <span class="sr-only">{LABEL[props.kind].desc}</span>
    </span>
  );
}
