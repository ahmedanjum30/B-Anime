import type { Signal } from "solid-js";
import { createStore, reconcile, unwrap } from "solid-js/store";

/**
 * Storage for a resource that is fetched again (after a sync, a scan): the new result is merged
 * into the old one (items matched by `key`), so a list whose items didn't change keeps its
 * cards and their pictures instead of drawing them all again (a blink on Home at every sync).
 * Pass as `createResource(fetcher, { storage: keepSameBy("id") })`.
 */
export const keepSameBy =
  (key: string) =>
  <T>(value: T | undefined): Signal<T | undefined> => {
    const [store, setStore] = createStore({ value });
    return [
      () => store.value,
      (v: unknown) => {
        const next = typeof v === "function" ? v(unwrap(store).value) : v;
        setStore("value", reconcile(next, { key }));
        return store.value;
      },
    ] as Signal<T | undefined>;
  };

/** `keepSameBy` for lists of shows (Home, Library, Schedule). */
export const keepSame = keepSameBy("mediaId");
