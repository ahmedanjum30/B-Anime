import { A } from "@solidjs/router";

/** Honest placeholder for screens not built yet. */
export default function Pending(props: { title: string }) {
  return (
    <div class="pending">
      <h1 class="section-head">{props.title}</h1>
      <p class="empty">
        This screen isn't built yet. It's on the Phase 1 build list. <A href="/">Back to Home</A>
      </p>
    </div>
  );
}
