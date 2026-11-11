import { createEffect, createSignal, on, onCleanup } from "solid-js";

import { deleteEntry, errorText, saveEntry } from "./api";
import type { EntryChange, ListEntry, ListStatus } from "./types";

const WATCHING: ListStatus[] = ["CURRENT", "REPEATING", "PLANNING", "PAUSED"];

/**
 * The list rules AniList users expect, applied before saving:
 * reaching the last episode completes the show, starting a planned show moves it to Watching,
 * and stepping back from Completed reopens it.
 */
export function applyRules(cur: ListEntry | null, change: EntryChange, total: number | null): EntryChange {
  const next: EntryChange = { ...change };
  const status = next.status ?? cur?.status ?? null;

  if (next.progress !== undefined) {
    next.progress = Math.max(0, total ? Math.min(next.progress, total) : next.progress);
    if (total && next.progress >= total && (!status || WATCHING.includes(status))) next.status = "COMPLETED";
    else if (next.progress > 0 && (!status || status === "PLANNING")) next.status = "CURRENT";
    else if (total && next.progress < total && status === "COMPLETED" && change.status === undefined) next.status = "CURRENT";
  }
  if (next.status === "COMPLETED" && total && next.progress === undefined) next.progress = total;
  if (!cur && !next.status) next.status = "PLANNING";
  return next;
}

/** Optimistic, debounced saving of one show's list entry. */
export function createEntrySaver(opts: {
  mediaId: () => number;
  total: () => number | null;
  initial: () => ListEntry | null;
  notify: (msg: string) => void;
}) {
  const [entry, setEntry] = createSignal<ListEntry | null>(opts.initial());
  const [state, setState] = createSignal<"idle" | "saving" | "saved" | "error">("idle");
  const [error, setError] = createSignal<string | null>(null);
  const [savedAt, setSavedAt] = createSignal<string | null>(null);

  let confirmed = opts.initial();
  let pending: EntryChange = {};
  let timer: number | undefined;

  createEffect(
    on(opts.initial, (e) => {
      confirmed = e;
      if (!Object.keys(pending).length && state() !== "saving") setEntry(e);
    }),
  );
  createEffect(on(opts.mediaId, () => {
    clearTimeout(timer);
    pending = {};
    setState("idle");
    setError(null);
  }, { defer: true }));
  onCleanup(() => {
    // Leaving the page with a debounced change still waiting: send it now.
    if (Object.keys(pending).length) void flush();
  });

  function save(change: EntryChange, debounceMs = 0) {
    const cur = entry();
    const next = applyRules(cur, change, opts.total());
    if (next.status === "COMPLETED" && cur?.status !== "COMPLETED") opts.notify("Marked as completed.");

    setEntry({
      status: next.status ?? cur?.status ?? "PLANNING",
      progress: next.progress ?? cur?.progress ?? 0,
      score: next.score !== undefined ? next.score : cur?.score ?? null,
      updatedAt: new Date().toISOString(),
    });
    pending = { ...pending, ...next };
    setError(null);
    clearTimeout(timer);
    timer = window.setTimeout(() => void flush(), debounceMs);
  }

  async function flush() {
    const change = pending;
    pending = {};
    if (!Object.keys(change).length) return;
    setState("saving");
    try {
      const saved = await saveEntry(opts.mediaId(), change);
      confirmed = saved;
      if (!Object.keys(pending).length) setEntry(saved);
      setSavedAt(saved.updatedAt);
      setState("saved");
    } catch (e) {
      setEntry(confirmed);
      setError(errorText(e));
      setState("error");
    }
  }

  /** Takes the show off the list on AniList. Any unsent edit is dropped first. */
  async function remove(): Promise<boolean> {
    clearTimeout(timer);
    pending = {};
    setState("saving");
    setError(null);
    try {
      await deleteEntry(opts.mediaId());
      confirmed = null;
      setEntry(null);
      setState("idle");
      opts.notify("Removed from your AniList list.");
      return true;
    } catch (e) {
      setError(errorText(e));
      setState("error");
      return false;
    }
  }

  return { entry, save, remove, state, error, savedAt };
}

export type EntrySaver = ReturnType<typeof createEntrySaver>;
